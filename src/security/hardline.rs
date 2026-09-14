use regex::Regex;
use std::sync::LazyLock;

use super::normalize::{
    command_detection_variants, command_parser_limit_exceeded, grep_safe_detection_variant,
    normalize_command_for_detection,
};

struct HardlineRule {
    regex: Regex,
    description: &'static str,
}

static SUDO_STDIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[;&|`\n]|&&|\|\||[$]\()\s*sudo\s+-S\b").unwrap());

fn build_hardline_rm_path(path_alt: &str) -> String {
    let tail = r"(?:\s|$|[)`;|&])";
    format!(r#"(?:['\x22](?:{path_alt})['\x22]|(?:{path_alt}){tail})"#)
}

static HARDLINE_PATTERNS: LazyLock<Vec<HardlineRule>> = LazyLock::new(|| {
    let cmdpos = r"(?:^|[\n`]|[$]\()\s*(?:sudo\s+(?:-[^\s]+\s+)*)?(?:env\s+(?:\w+=\S*\s+)*)?(?:(?:exec|nohup|setsid|time)\s+)*\s*";
    let rm_prefix = format!(r"{cmdpos}rm\s+(-[^\s]*\s+)*");
    let hardline_system_dirs = r"/home|/home/\*|/root|/root/\*|/etc|/etc/\*|/usr|/usr/\*|/var|/var/\*|/bin|/bin/\*|/sbin|/sbin/\*|/boot|/boot/\*|/lib|/lib/\*|/System|/System/\*";

    let root_rm_path = build_hardline_rm_path(r"/(?:/|[.]/|[.][.]/)*(?:[.]|[.][.])?\**|/ \*");
    let sys_rm_path = build_hardline_rm_path(hardline_system_dirs);
    let home_rm_path = build_hardline_rm_path(r"(?:~|\$\{?HOME\}?)(?:/?|/\*)?");

    let raw_rules = vec![
        // rm root / system / home
        (
            format!(r"{rm_prefix}{root_rm_path}"),
            "recursive delete of root filesystem",
        ),
        (
            format!(r"{rm_prefix}{sys_rm_path}"),
            "recursive delete of system directory",
        ),
        (
            format!(r"{rm_prefix}{home_rm_path}"),
            "recursive delete of home directory",
        ),
        // Filesystem format
        (
            r"\bmkfs(\.[a-z0-9]+)?\b".to_string(),
            "format filesystem (mkfs)",
        ),
        // Raw block device overwrites
        (
            r"\bdd\b[^\n]*\bof=/dev/(sd|nvme|hd|mmcblk|vd|xvd|disk)[a-z0-9]*".to_string(),
            "dd to raw block device",
        ),
        (
            r">\s*/dev/(sd|nvme|hd|mmcblk|vd|xvd|disk)[a-z0-9]*\b".to_string(),
            "redirect to raw block device",
        ),
        // Fork bomb
        (
            r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:".to_string(),
            "fork bomb",
        ),
        // Kill all processes
        (
            r"\bkill\s+(-[^\s]+\s+)*-1\b".to_string(),
            "kill all processes",
        ),
        (r"\bkill\s+-9\s+-1\b".to_string(), "kill all processes"),
        // Shutdown / reboot / halt
        (
            format!(r"{cmdpos}(shutdown|reboot|halt|poweroff)\b"),
            "system shutdown/reboot",
        ),
        (
            format!(r"{cmdpos}init\s+[06]\b"),
            "init 0/6 (shutdown/reboot)",
        ),
        (
            format!(r"{cmdpos}systemctl\s+(poweroff|reboot|halt|kexec)\b"),
            "systemctl poweroff/reboot",
        ),
        (
            format!(r"{cmdpos}telinit\s+[06]\b"),
            "telinit 0/6 (shutdown/reboot)",
        ),
    ];

    raw_rules
        .into_iter()
        .map(|(pat, desc)| {
            let regex = regex::RegexBuilder::new(&pat)
                .case_insensitive(true)
                .dot_matches_new_line(true)
                .build()
                .unwrap_or_else(|e| panic!("invalid hardline regex {pat}: {e}"));
            HardlineRule {
                regex,
                description: desc,
            }
        })
        .collect()
});

pub fn check_sudo_stdin_guard(command: &str) -> Option<String> {
    if std::env::var("SUDO_PASSWORD").is_ok() {
        return None;
    }
    let normalized = normalize_command_for_detection(command);
    if SUDO_STDIN_RE.is_match(&normalized) {
        return Some("sudo password guessing via stdin (sudo -S)".to_string());
    }
    None
}

pub fn detect_hardline_command(command: &str) -> Option<String> {
    if command_parser_limit_exceeded(command) {
        return Some("command parser limit exceeded".to_string());
    }
    let normalized = normalize_command_for_detection(command);
    let (_, malformed_grep) = grep_safe_detection_variant(&normalized);
    if malformed_grep {
        return Some("command parser limit or malformed executable payload".to_string());
    }
    for variant in command_detection_variants(command) {
        let variant_lower = variant.to_ascii_lowercase();
        for rule in HARDLINE_PATTERNS.iter() {
            if rule.regex.is_match(&variant_lower) {
                return Some(rule.description.to_string());
            }
        }
    }
    if let Some(reason) = check_sudo_stdin_guard(command) {
        return Some(reason);
    }
    None
}

/// Matches wildcard patterns (fnmatch style) where `*` matches any sequence of characters and `?` matches any single character.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    let (p_len, t_len) = (p_chars.len(), t_chars.len());
    let mut dp = vec![vec![false; t_len + 1]; p_len + 1];
    dp[0][0] = true;
    for i in 1..=p_len {
        if p_chars[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        } else {
            break;
        }
    }
    for i in 1..=p_len {
        for j in 1..=t_len {
            if p_chars[i - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if p_chars[i - 1] == '?' || p_chars[i - 1] == t_chars[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[p_len][t_len]
}

/// Returns the matching `APPROVALS_DENY` pattern if the command or any normalized variant matches.
pub fn match_user_deny_rule<'a>(command: &str, deny_patterns: &'a [String]) -> Option<&'a str> {
    if deny_patterns.is_empty() {
        return None;
    }
    let globs: Vec<&'a str> = deny_patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if globs.is_empty() {
        return None;
    }
    for variant in command_detection_variants(command) {
        let candidate = variant.trim();
        for pattern in &globs {
            if wildcard_match(pattern, candidate) {
                return Some(pattern);
            }
        }
        // A deny glob names an executable and its arguments. Matching only the whole
        // variant lets a wrapper prefix (env / sudo / exec / nohup, env assignments)
        // launder the very command the operator unconditionally denied, so match the
        // wrapper-stripped executable form as well.
        for stripped in wrapper_stripped_forms(candidate) {
            for pattern in &globs {
                if wildcard_match(pattern, stripped.trim()) {
                    return Some(pattern);
                }
            }
        }
    }
    None
}

/// Yields each command-start span of `command` re-rooted at its real executable, i.e.
/// with wrapper prefixes such as `env`, `sudo`, `exec` and leading `VAR=value`
/// assignments removed. Returns nothing when there was no prefix to strip.
fn wrapper_stripped_forms(command: &str) -> Vec<String> {
    let mut forms = Vec::new();
    for (start, _, _) in crate::security::normalize::iter_shell_command_word_spans(command) {
        if start == 0 {
            continue;
        }
        let candidate = command[start..].trim();
        if !candidate.is_empty() && !forms.iter().any(|f| f == candidate) {
            forms.push(candidate.to_string());
        }
    }
    forms
}

#[cfg(test)]
mod deny_bypass_tests {
    use super::match_user_deny_rule;

    #[test]
    fn wrapper_prefixed_commands_still_trip_a_deny_glob() {
        let deny = vec!["npm publish *".to_string()];

        // Baseline: the bare form is denied today.
        assert_eq!(
            match_user_deny_rule("npm publish --access public", &deny),
            Some("npm publish *"),
            "bare command must be denied"
        );

        // Wrapper prefixes must not launder the same command past the operator's deny rule.
        for wrapped in [
            "env npm publish --access public",
            "sudo npm publish --access public",
            "exec npm publish --access public",
            "nohup npm publish --access public",
            "env FOO=bar npm publish --access public",
        ] {
            assert_eq!(
                match_user_deny_rule(wrapped, &deny),
                Some("npm publish *"),
                "wrapper-prefixed command must still be denied: {wrapped}"
            );
        }
    }
}
