use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};

use super::Tool;
use crate::OmonError;

#[derive(Clone, Default)]
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the live web for current information, documentation, news, or facts using DuckDuckGo search API."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to execute."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of search results to return (default 5)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value, OmonError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| OmonError::ToolExecution("missing 'query'".into()))?;
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(bounded_redirect_policy())
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .map_err(|e| OmonError::ToolExecution(e.to_string()))?;

        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| OmonError::ToolExecution(format!("search request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(OmonError::ToolExecution(format!(
                "search provider returned {}",
                resp.status()
            )));
        }
        let html = read_bounded_text(resp.bytes_stream(), MAX_SEARCH_CHARS).await?;

        let mut results = Vec::new();
        for snippet in html
            .split("<div class=\"result__body\">")
            .skip(1)
            .take(max_results)
        {
            let title = extract_tag_content(snippet, "<a class=\"result__url\"", "</a>")
                .or_else(|| extract_tag_content(snippet, "<a class=\"result__snippet\"", "</a>"))
                .unwrap_or_default();
            let body = extract_tag_content(snippet, "<a class=\"result__snippet\"", "</a>")
                .unwrap_or_default();
            let href = extract_attribute(snippet, "href=\"", "\"").unwrap_or_default();

            if !title.is_empty() || !body.is_empty() {
                results.push(json!({
                    "title": clean_html(&title),
                    "snippet": clean_html(&body),
                    "url": href
                }));
            }
        }

        if results.is_empty() {
            return Err(OmonError::ToolExecution(
                "search provider response contained no parseable results".into(),
            ));
        }

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results
        }))
    }
}

#[derive(Clone, Default)]
pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL content and extract clean readable text/markdown from any webpage."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 8000)."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value, OmonError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| OmonError::ToolExecution("missing 'url'".into()))?;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(100_000)
            .clamp(1, 1_000_000) as usize;
        let parsed = reqwest::Url::parse(url)
            .map_err(|error| OmonError::ToolExecution(format!("invalid URL: {error}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(OmonError::ToolExecution(
                "web fetch only supports http and https URLs".into(),
            ));
        }
        let reader_prefix = std::env::var(READER_PREFIX_ENV).ok();
        let fetch_url = fetch_target_url(&parsed, reader_prefix.as_deref());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(bounded_redirect_policy())
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .map_err(|e| OmonError::ToolExecution(e.to_string()))?;

        let resp = client
            .get(&fetch_url)
            .send()
            .await
            .map_err(|e| OmonError::ToolExecution(format!("fetch request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(OmonError::ToolExecution(format!(
                "fetch provider returned {}",
                resp.status()
            )));
        }
        let truncated = read_bounded_text(resp.bytes_stream(), max_chars).await?;

        Ok(json!({
            "url": url,
            "length": truncated.chars().count(),
            "content": truncated
        }))
    }
}

fn extract_tag_content(html: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let start = html.find(start_tag)?;
    let rest = &html[start..];
    let content_start = rest.find('>')? + 1;
    let end = rest[content_start..].find(end_tag)?;
    Some(rest[content_start..content_start + end].to_string())
}

fn extract_attribute(html: &str, attr_prefix: &str, end_char: &str) -> Option<String> {
    let start = html.find(attr_prefix)? + attr_prefix.len();
    let rest = &html[start..];
    let end = rest.find(end_char)?;
    Some(rest[..end].to_string())
}

const MAX_REDIRECTS: usize = 5;
/// The raw page is fetched directly by default so that no third party learns
/// which URLs (including internal ones) the agent reads. A reader/proxy service
/// is opt-in: set `OMON_WEB_READER_PREFIX` (e.g. `https://r.jina.ai/`) to route
/// fetches through it, accepting that the operator discloses every fetched URL
/// to that service and lets it control the text injected into model context.
const READER_PREFIX_ENV: &str = "OMON_WEB_READER_PREFIX";
const MAX_SEARCH_CHARS: usize = 500_000;

fn fetch_target_url(parsed: &reqwest::Url, reader_prefix: Option<&str>) -> String {
    match reader_prefix {
        Some(prefix) if !prefix.trim().is_empty() => format!("{}{parsed}", prefix.trim()),
        _ => parsed.to_string(),
    }
}

fn redirect_is_allowed(next: &reqwest::Url, hops: usize) -> bool {
    hops <= MAX_REDIRECTS && matches!(next.scheme(), "http" | "https")
}

/// Every redirect hop is re-validated: the scheme check on the original URL
/// only ever covers hop zero.
fn bounded_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if redirect_is_allowed(attempt.url(), attempt.previous().len()) {
            attempt.follow()
        } else {
            let message = format!(
                "blocked redirect to {} after {} hops",
                attempt.url(),
                attempt.previous().len()
            );
            attempt.error(message)
        }
    })
}

/// Streams the body and stops as soon as `max_chars` characters have been
/// decoded, so an oversized response is never fully buffered. Bytes are decoded
/// incrementally at character boundaries; a multi-byte sequence split across two
/// network chunks is held until it completes instead of being corrupted.
async fn read_bounded_text<S, B, E>(stream: S, max_chars: usize) -> Result<String, OmonError>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut stream = std::pin::pin!(stream);
    let mut pending: Vec<u8> = Vec::new();
    let mut text = String::new();
    let mut remaining = max_chars;
    while remaining > 0 {
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk.map_err(|error| {
            OmonError::ToolExecution(format!("failed to read response: {error}"))
        })?;
        pending.extend_from_slice(chunk.as_ref());
        push_decoded(&mut pending, &mut text, &mut remaining);
    }
    if remaining > 0 && !pending.is_empty() {
        // Trailing truncated sequence at end of stream.
        text.push(char::REPLACEMENT_CHARACTER);
    }
    Ok(text)
}

fn push_decoded(pending: &mut Vec<u8>, out: &mut String, remaining: &mut usize) {
    let mut consumed = 0usize;
    while consumed < pending.len() && *remaining > 0 {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(valid) => {
                consumed += push_chars(valid, out, remaining);
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                let valid =
                    std::str::from_utf8(&pending[consumed..consumed + valid_up_to]).unwrap_or("");
                let taken = push_chars(valid, out, remaining);
                consumed += taken;
                if taken < valid_up_to {
                    break;
                }
                match error.error_len() {
                    Some(len) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        *remaining -= 1;
                        consumed += len;
                    }
                    // Incomplete sequence at the chunk boundary: wait for more bytes.
                    None => break,
                }
            }
        }
    }
    pending.drain(..consumed);
}

fn push_chars(input: &str, out: &mut String, remaining: &mut usize) -> usize {
    let mut used = 0usize;
    for character in input.chars() {
        if *remaining == 0 {
            break;
        }
        out.push(character);
        *remaining -= 1;
        used += character.len_utf8();
    }
    used
}

fn clean_html(input: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in input.chars() {
        if c == '<' {
            inside = true;
        } else if c == '>' {
            inside = false;
        } else if !inside {
            out.push(c);
        }
    }
    out.replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn web_fetch_targets_url_directly_unless_reader_configured() {
        let parsed = reqwest::Url::parse("https://internal.corp.example/secret").unwrap();

        assert_eq!(
            fetch_target_url(&parsed, None),
            "https://internal.corp.example/secret",
            "default fetch must not disclose the URL to a third-party reader service"
        );
        assert_eq!(
            fetch_target_url(&parsed, Some("https://r.jina.ai/")),
            "https://r.jina.ai/https://internal.corp.example/secret",
            "an explicitly configured reader service is still honoured"
        );
    }

    #[test]
    fn redirect_hops_revalidate_scheme_and_are_capped() {
        let https = reqwest::Url::parse("https://example.com/a").unwrap();
        let http = reqwest::Url::parse("http://example.com/a").unwrap();
        let file = reqwest::Url::parse("file:///etc/passwd").unwrap();

        assert!(redirect_is_allowed(&https, 1));
        assert!(redirect_is_allowed(&http, 1));
        assert!(
            !redirect_is_allowed(&file, 1),
            "redirect hops must re-validate the scheme"
        );
        assert!(
            !redirect_is_allowed(&https, MAX_REDIRECTS + 1),
            "redirect chains must be capped"
        );
    }

    #[tokio::test]
    async fn read_bounded_text_stops_reading_once_max_chars_reached() {
        let polled = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&polled);
        let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
            (0..100).map(|_| Ok(vec![b'a'; 1024])).collect();
        let stream = futures_util::stream::iter(chunks).map(move |item| {
            counter.fetch_add(1, Ordering::SeqCst);
            item
        });

        let text = read_bounded_text(stream, 2048).await.unwrap();

        assert_eq!(text.chars().count(), 2048);
        assert!(
            polled.load(Ordering::SeqCst) < 100,
            "body must stop streaming at max_chars, polled {} of 100 chunks",
            polled.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn read_bounded_text_decodes_multibyte_split_across_chunks() {
        let encoded = "한글".as_bytes().to_vec();
        let (head, tail) = encoded.split_at(4);
        let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
            vec![Ok(head.to_vec()), Ok(tail.to_vec())];

        let text = read_bounded_text(futures_util::stream::iter(chunks), 100)
            .await
            .unwrap();

        assert_eq!(text, "한글");
    }
}
