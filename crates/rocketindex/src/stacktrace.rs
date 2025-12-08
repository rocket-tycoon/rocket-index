//! Stacktrace parsing for multiple languages.
//!
//! Parses stacktrace text and extracts structured frame information
//! for use with symbol enrichment.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single frame extracted from a stacktrace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFrame {
    /// The symbol name (e.g., "UserService.getUser", "process_payment")
    pub symbol: String,
    /// The source file path, if present in the stacktrace
    pub file: Option<PathBuf>,
    /// The line number, if present
    pub line: Option<u32>,
    /// The column number, if present
    pub column: Option<u32>,
    /// Whether this appears to be user code (not framework/library)
    pub is_user_code: bool,
    /// The detected language of this frame
    pub language: Option<StacktraceLanguage>,
}

/// Supported stacktrace languages/formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StacktraceLanguage {
    Java,
    Ruby,
    Python,
    JavaScript,
    Rust,
    Go,
}

impl std::fmt::Display for StacktraceLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StacktraceLanguage::Java => write!(f, "java"),
            StacktraceLanguage::Ruby => write!(f, "ruby"),
            StacktraceLanguage::Python => write!(f, "python"),
            StacktraceLanguage::JavaScript => write!(f, "javascript"),
            StacktraceLanguage::Rust => write!(f, "rust"),
            StacktraceLanguage::Go => write!(f, "go"),
        }
    }
}

/// Result of parsing a stacktrace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StacktraceResult {
    /// Parsed frames in order (top of stack first)
    pub frames: Vec<StackFrame>,
    /// The detected primary language of the stacktrace
    pub detected_language: Option<StacktraceLanguage>,
    /// Lines that couldn't be parsed as frames
    pub unparsed_lines: Vec<String>,
}

/// Parse a stacktrace string into structured frames.
///
/// Automatically detects the language format and extracts frames.
///
/// # Example
/// ```
/// use rocketindex::stacktrace::parse_stacktrace;
///
/// let trace = r#"
/// java.lang.NullPointerException: null
///     at com.example.UserService.getUser(UserService.java:42)
///     at com.example.Controller.show(Controller.java:15)
/// "#;
///
/// let result = parse_stacktrace(trace);
/// assert_eq!(result.frames.len(), 2);
/// assert_eq!(result.frames[0].symbol, "com.example.UserService.getUser");
/// ```
pub fn parse_stacktrace(text: &str) -> StacktraceResult {
    let mut result = StacktraceResult::default();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip exception headers first
        if is_exception_header(trimmed) {
            continue;
        }

        // Try each language parser in order of specificity
        // More specific patterns first to avoid false positives
        if let Some(frame) = try_parse_rust(trimmed) {
            // Rust: "N: symbol" or "at path:line:col"
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::Rust);
            }
            result.frames.push(frame);
        } else if let Some(frame) = try_parse_python(trimmed) {
            // Python: File "path", line N, in func
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::Python);
            }
            result.frames.push(frame);
        } else if let Some(frame) = try_parse_ruby(trimmed) {
            // Ruby: path:line:in `method'
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::Ruby);
            }
            result.frames.push(frame);
        } else if let Some(frame) = try_parse_java(trimmed) {
            // Java: at package.Class.method(File.java:line)
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::Java);
            }
            result.frames.push(frame);
        } else if let Some(frame) = try_parse_javascript(trimmed) {
            // JS: at func (path:line:col) or at path:line:col
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::JavaScript);
            }
            result.frames.push(frame);
        } else if let Some(frame) = try_parse_go(trimmed) {
            // Go: func(args) or path:line +0x...
            if result.detected_language.is_none() {
                result.detected_language = Some(StacktraceLanguage::Go);
            }
            result.frames.push(frame);
        } else {
            result.unparsed_lines.push(trimmed.to_string());
        }
    }

    result
}

/// Check if a line is an exception header (not a frame)
fn is_exception_header(line: &str) -> bool {
    // Java: "java.lang.NullPointerException: message"
    // Python: "Traceback (most recent call last):"
    // Ruby: "/path/file.rb:42:in `method': message (ErrorClass)"
    // JS: "Error: message" or "TypeError: message"
    // Go: "goroutine N [running]:"
    line.starts_with("Traceback")
        || line.starts_with("Caused by:")
        || line.starts_with("panic:")
        || line.starts_with("goroutine ")
        || line.starts_with("thread '")
        // Exception patterns - but not if it looks like a frame
        || (line.contains("Exception:") && !line.trim().starts_with("at "))
        || (line.contains("Error:") && !line.trim().starts_with("at "))
}

// Default framework patterns for common languages
const JAVA_FRAMEWORK_PREFIXES: &[&str] = &[
    "java.",
    "javax.",
    "sun.",
    "com.sun.",
    "org.springframework.",
    "org.apache.",
    "org.hibernate.",
    "io.netty.",
];

const RUBY_FRAMEWORK_PATTERNS: &[&str] = &[
    "/gems/",
    "/ruby/",
    "/bundler/",
    "rails/",
    "activerecord",
    "activesupport",
    "actionpack",
];

const PYTHON_FRAMEWORK_PATTERNS: &[&str] = &[
    "site-packages/",
    "/lib/python",
    "django/",
    "flask/",
    "werkzeug/",
    "celery/",
];

const JS_FRAMEWORK_PATTERNS: &[&str] = &[
    "node_modules/",
    "internal/",
    "timers.js",
    "events.js",
    "module.js",
];

const RUST_FRAMEWORK_PATTERNS: &[&str] = &[
    "std::",
    "core::",
    "alloc::",
    "tokio::",
    "hyper::",
    "<unknown>",
];

const GO_FRAMEWORK_PATTERNS: &[&str] =
    &["runtime.", "runtime/", "net/http.", "syscall.", "internal/"];

fn is_framework_code(symbol: &str, file: Option<&PathBuf>, language: StacktraceLanguage) -> bool {
    let patterns = match language {
        StacktraceLanguage::Java => JAVA_FRAMEWORK_PREFIXES,
        StacktraceLanguage::Ruby => RUBY_FRAMEWORK_PATTERNS,
        StacktraceLanguage::Python => PYTHON_FRAMEWORK_PATTERNS,
        StacktraceLanguage::JavaScript => JS_FRAMEWORK_PATTERNS,
        StacktraceLanguage::Rust => RUST_FRAMEWORK_PATTERNS,
        StacktraceLanguage::Go => GO_FRAMEWORK_PATTERNS,
    };

    // Check symbol name
    for pattern in patterns {
        if symbol.contains(pattern) {
            return true;
        }
    }

    // Check file path if available
    if let Some(path) = file {
        let path_str = path.to_string_lossy().to_lowercase();
        for pattern in patterns {
            if path_str.contains(&pattern.to_lowercase()) {
                return true;
            }
        }
    }

    false
}

/// Parse a Java stacktrace line.
/// Format: "at com.example.Class.method(File.java:42)"
fn try_parse_java(line: &str) -> Option<StackFrame> {
    // Must start with "at " (with some possible leading whitespace already trimmed)
    let line = line.strip_prefix("at ")?;

    // Find the opening paren for file info
    let paren_start = line.find('(')?;
    let symbol = line[..paren_start].to_string();

    // Java symbols should look like package.Class.method (dots, no spaces before paren)
    // This distinguishes from JS's "at method (file:line:col)" format
    if !symbol.contains('.') || symbol.contains(' ') {
        return None;
    }

    // Extract file:line from parens
    let paren_end = line.find(')')?;
    let file_info = &line[paren_start + 1..paren_end];

    // Java files end in .java, .kt, .scala, .groovy, or are special markers
    let is_java_file = file_info.ends_with(".java")
        || file_info.ends_with(".kt")
        || file_info.ends_with(".scala")
        || file_info.ends_with(".groovy")
        || file_info.contains(".java:")
        || file_info.contains(".kt:")
        || file_info.contains(".scala:")
        || file_info.contains(".groovy:")
        || file_info == "Native Method"
        || file_info == "Unknown Source";

    if !is_java_file {
        return None;
    }

    let (file, line_num) = if file_info.contains(':') {
        let parts: Vec<&str> = file_info.rsplitn(2, ':').collect();
        let line_num = parts[0].parse::<u32>().ok();
        let file = Some(PathBuf::from(parts.get(1).unwrap_or(&"")));
        (file, line_num)
    } else if file_info == "Native Method" || file_info == "Unknown Source" {
        (None, None)
    } else {
        (Some(PathBuf::from(file_info)), None)
    };

    let is_user_code = !is_framework_code(&symbol, file.as_ref(), StacktraceLanguage::Java);

    Some(StackFrame {
        symbol,
        file,
        line: line_num,
        column: None,
        is_user_code,
        language: Some(StacktraceLanguage::Java),
    })
}

/// Parse a Ruby stacktrace line.
/// Format: "from /path/file.rb:42:in `method'"
/// or: "/path/file.rb:42:in `method'"
fn try_parse_ruby(line: &str) -> Option<StackFrame> {
    // Remove leading "from " if present
    let line = line.strip_prefix("from ").unwrap_or(line);

    // Look for the pattern: path:line:in `method'
    // Use regex-like manual parsing
    let in_pos = line.find(":in `")?;
    let method_end = line.rfind('\'')?;

    if method_end <= in_pos + 5 {
        return None;
    }

    let method = line[in_pos + 5..method_end].to_string();

    // Parse file:line before ":in"
    let file_line_part = &line[..in_pos];
    let colon_pos = file_line_part.rfind(':')?;
    let line_num = file_line_part[colon_pos + 1..].parse::<u32>().ok()?;
    let file = PathBuf::from(&file_line_part[..colon_pos]);

    let is_user_code = !is_framework_code(&method, Some(&file), StacktraceLanguage::Ruby);

    Some(StackFrame {
        symbol: method,
        file: Some(file),
        line: Some(line_num),
        column: None,
        is_user_code,
        language: Some(StacktraceLanguage::Ruby),
    })
}

/// Parse a Python stacktrace line.
/// Format: File "/path/file.py", line 42, in method
fn try_parse_python(line: &str) -> Option<StackFrame> {
    // Must start with "File "
    let line = line.strip_prefix("File ")?;

    // Find the quoted file path
    let quote_start = line.find('"')?;
    let quote_end = line[quote_start + 1..].find('"')? + quote_start + 1;
    let file = PathBuf::from(&line[quote_start + 1..quote_end]);

    // Find ", line N"
    let after_file = &line[quote_end + 1..];
    let line_marker = ", line ";
    let line_pos = after_file.find(line_marker)?;
    let after_line_marker = &after_file[line_pos + line_marker.len()..];

    // Parse line number (ends at comma or end of string)
    let line_end = after_line_marker
        .find(',')
        .unwrap_or(after_line_marker.len());
    let line_num = after_line_marker[..line_end].parse::<u32>().ok()?;

    // Find ", in method"
    let in_marker = ", in ";
    let method = if let Some(in_pos) = after_line_marker.find(in_marker) {
        after_line_marker[in_pos + in_marker.len()..]
            .trim()
            .to_string()
    } else {
        "<module>".to_string()
    };

    let is_user_code = !is_framework_code(&method, Some(&file), StacktraceLanguage::Python);

    Some(StackFrame {
        symbol: method,
        file: Some(file),
        line: Some(line_num),
        column: None,
        is_user_code,
        language: Some(StacktraceLanguage::Python),
    })
}

/// Parse a JavaScript/Node.js stacktrace line.
/// Format: "at method (/path/file.js:42:15)"
/// or: "at /path/file.js:42:15"
fn try_parse_javascript(line: &str) -> Option<StackFrame> {
    let line = line.strip_prefix("at ")?;

    // Check for format: "method (file:line:col)"
    if let Some(paren_start) = line.find('(') {
        let method = line[..paren_start].trim().to_string();
        let paren_end = line.find(')')?;
        let file_info = &line[paren_start + 1..paren_end];

        let (file, line_num, column) = parse_js_location(file_info)?;
        let is_user_code = !is_framework_code(&method, Some(&file), StacktraceLanguage::JavaScript);

        Some(StackFrame {
            symbol: method,
            file: Some(file),
            line: Some(line_num),
            column,
            is_user_code,
            language: Some(StacktraceLanguage::JavaScript),
        })
    } else {
        // Format: "file:line:col" (anonymous)
        let (file, line_num, column) = parse_js_location(line)?;
        let is_user_code =
            !is_framework_code("<anonymous>", Some(&file), StacktraceLanguage::JavaScript);

        Some(StackFrame {
            symbol: "<anonymous>".to_string(),
            file: Some(file),
            line: Some(line_num),
            column,
            is_user_code,
            language: Some(StacktraceLanguage::JavaScript),
        })
    }
}

fn parse_js_location(s: &str) -> Option<(PathBuf, u32, Option<u32>)> {
    // Format: /path/file.js:42:15 or /path/file.js:42
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    match parts.len() {
        3 => {
            let col = parts[0].parse::<u32>().ok();
            let line = parts[1].parse::<u32>().ok()?;
            let file = PathBuf::from(parts[2]);
            Some((file, line, col))
        }
        2 => {
            let line = parts[0].parse::<u32>().ok()?;
            let file = PathBuf::from(parts[1]);
            Some((file, line, None))
        }
        _ => None,
    }
}

/// Parse a Rust stacktrace line.
/// Format: "   N: module::function"
/// or: "             at /path/file.rs:42:5"
fn try_parse_rust(line: &str) -> Option<StackFrame> {
    // Numbered frame: "   0: tokio::runtime::task::harness::poll"
    // Must have "::" in symbol to be Rust (distinguishes from Go's "main.handler")
    if let Some(colon_pos) = line.find(": ") {
        let before_colon = line[..colon_pos].trim();
        if before_colon.parse::<u32>().is_ok() {
            let symbol = line[colon_pos + 2..].trim().to_string();
            // Rust symbols use :: as separator - this distinguishes from Go
            if !symbol.contains("::") {
                return None;
            }
            let is_user_code = !is_framework_code(&symbol, None, StacktraceLanguage::Rust);

            return Some(StackFrame {
                symbol,
                file: None,
                line: None,
                column: None,
                is_user_code,
                language: Some(StacktraceLanguage::Rust),
            });
        }
    }

    // Location line: "             at /path/file.rs:42:5"
    // Must end in .rs to be Rust
    let trimmed = line.trim();
    let after_at = trimmed.strip_prefix("at ")?;
    if !after_at.contains(".rs:") {
        return None;
    }
    let (file, line_num, column) = parse_js_location(after_at)?;

    Some(StackFrame {
        symbol: "<location>".to_string(),
        file: Some(file),
        line: Some(line_num),
        column,
        is_user_code: true, // Location lines are typically user code
        language: Some(StacktraceLanguage::Rust),
    })
}

/// Parse a Go stacktrace line.
/// Format: "main.handler(0x1234)"
/// or: "        /path/file.go:42 +0x1a"
fn try_parse_go(line: &str) -> Option<StackFrame> {
    let trimmed = line.trim();

    // Check for function line: "package.function(args)"
    if trimmed.contains('(') && !trimmed.starts_with('/') && !trimmed.starts_with('.') {
        let paren_pos = trimmed.find('(')?;
        let symbol = trimmed[..paren_pos].to_string();

        // Skip if it looks like a file path
        if symbol.contains('/') && !symbol.contains('.') {
            return None;
        }

        let is_user_code = !is_framework_code(&symbol, None, StacktraceLanguage::Go);

        return Some(StackFrame {
            symbol,
            file: None,
            line: None,
            column: None,
            is_user_code,
            language: Some(StacktraceLanguage::Go),
        });
    }

    // Check for location line: "/path/file.go:42 +0x1a"
    if trimmed.starts_with('/') || trimmed.starts_with('.') {
        // Remove the +0x... suffix if present
        let line_without_offset = if let Some(plus_pos) = trimmed.rfind(" +0x") {
            &trimmed[..plus_pos]
        } else {
            trimmed
        };

        // Parse file:line
        let colon_pos = line_without_offset.rfind(':')?;
        let line_num = line_without_offset[colon_pos + 1..].parse::<u32>().ok()?;
        let file = PathBuf::from(&line_without_offset[..colon_pos]);

        return Some(StackFrame {
            symbol: "<location>".to_string(),
            file: Some(file),
            line: Some(line_num),
            column: None,
            is_user_code: true,
            language: Some(StacktraceLanguage::Go),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============= Java Tests =============

    #[test]
    fn test_java_basic_stacktrace() {
        let trace = r#"
java.lang.NullPointerException: null
    at com.va.gov.UserService.getUser(UserService.java:42)
    at com.va.gov.Controller.show(Controller.java:15)
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 2);
        assert_eq!(result.detected_language, Some(StacktraceLanguage::Java));

        assert_eq!(result.frames[0].symbol, "com.va.gov.UserService.getUser");
        assert_eq!(
            result.frames[0].file,
            Some(PathBuf::from("UserService.java"))
        );
        assert_eq!(result.frames[0].line, Some(42));
        assert!(result.frames[0].is_user_code);
    }

    #[test]
    fn test_java_framework_detection() {
        let trace = r#"
    at org.springframework.web.servlet.DispatcherServlet.doDispatch(DispatcherServlet.java:1067)
    at javax.servlet.http.HttpServlet.service(HttpServlet.java:750)
    at com.va.gov.MyHandler.handle(MyHandler.java:25)
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 3);
        assert!(!result.frames[0].is_user_code); // Spring
        assert!(!result.frames[1].is_user_code); // javax
        assert!(result.frames[2].is_user_code); // User code
    }

    #[test]
    fn test_java_native_method() {
        let trace = "    at sun.reflect.NativeMethodAccessorImpl.invoke0(Native Method)";
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].file, None);
        assert_eq!(result.frames[0].line, None);
    }

    // ============= Ruby Tests =============

    #[test]
    fn test_ruby_basic_stacktrace() {
        let trace = r#"
/app/services/user_service.rb:42:in `get_user'
/app/controllers/users_controller.rb:15:in `show'
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 2);
        assert_eq!(result.detected_language, Some(StacktraceLanguage::Ruby));

        assert_eq!(result.frames[0].symbol, "get_user");
        assert_eq!(
            result.frames[0].file,
            Some(PathBuf::from("/app/services/user_service.rb"))
        );
        assert_eq!(result.frames[0].line, Some(42));
    }

    #[test]
    fn test_ruby_with_from_prefix() {
        let trace = "from /app/models/user.rb:10:in `validate'";
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].symbol, "validate");
    }

    #[test]
    fn test_ruby_framework_detection() {
        let trace = r#"
/home/user/.rvm/gems/ruby-3.0.0/gems/activerecord-7.0.0/lib/active_record/base.rb:100:in `find'
/app/models/user.rb:25:in `fetch_user'
"#;
        let result = parse_stacktrace(trace);

        assert!(!result.frames[0].is_user_code); // ActiveRecord gem
        assert!(result.frames[1].is_user_code); // User code
    }

    // ============= Python Tests =============

    #[test]
    fn test_python_basic_stacktrace() {
        let trace = r#"
Traceback (most recent call last):
  File "/app/services/user_service.py", line 42, in get_user
  File "/app/views.py", line 15, in show
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 2);
        assert_eq!(result.detected_language, Some(StacktraceLanguage::Python));

        assert_eq!(result.frames[0].symbol, "get_user");
        assert_eq!(
            result.frames[0].file,
            Some(PathBuf::from("/app/services/user_service.py"))
        );
        assert_eq!(result.frames[0].line, Some(42));
    }

    #[test]
    fn test_python_framework_detection() {
        let trace = r#"
  File "/usr/lib/python3.9/site-packages/django/views/generic/base.py", line 98, in dispatch
  File "/app/views.py", line 25, in get
"#;
        let result = parse_stacktrace(trace);

        assert!(!result.frames[0].is_user_code); // Django
        assert!(result.frames[1].is_user_code); // User code
    }

    // ============= JavaScript Tests =============

    #[test]
    fn test_javascript_basic_stacktrace() {
        let trace = r#"
Error: Something went wrong
    at UserService.getUser (/app/services/userService.js:42:15)
    at Controller.show (/app/controllers/userController.js:15:10)
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 2);
        assert_eq!(
            result.detected_language,
            Some(StacktraceLanguage::JavaScript)
        );

        assert_eq!(result.frames[0].symbol, "UserService.getUser");
        assert_eq!(
            result.frames[0].file,
            Some(PathBuf::from("/app/services/userService.js"))
        );
        assert_eq!(result.frames[0].line, Some(42));
        assert_eq!(result.frames[0].column, Some(15));
    }

    #[test]
    fn test_javascript_anonymous() {
        let trace = "    at /app/index.js:10:5";
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].symbol, "<anonymous>");
        assert_eq!(result.frames[0].line, Some(10));
    }

    #[test]
    fn test_javascript_framework_detection() {
        let trace = r#"
    at Module._compile (internal/modules/cjs/loader.js:1085:14)
    at processTicksAndRejections (node:internal/process/task_queues:95:5)
    at myHandler (/app/handler.js:25:10)
"#;
        let result = parse_stacktrace(trace);

        assert!(!result.frames[0].is_user_code); // Node internal
        assert!(!result.frames[1].is_user_code); // Node internal
        assert!(result.frames[2].is_user_code); // User code
    }

    // ============= Rust Tests =============

    #[test]
    fn test_rust_basic_stacktrace() {
        let trace = r#"
thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value'
   0: my_app::handlers::user::get_user
   1: my_app::main
             at ./src/main.rs:42:5
"#;
        let result = parse_stacktrace(trace);

        assert!(result.frames.len() >= 2);
        assert_eq!(result.detected_language, Some(StacktraceLanguage::Rust));

        assert_eq!(result.frames[0].symbol, "my_app::handlers::user::get_user");
        assert!(result.frames[0].is_user_code);
    }

    #[test]
    fn test_rust_framework_detection() {
        let trace = r#"
   0: std::panicking::begin_panic
   1: core::result::unwrap_failed
   2: tokio::runtime::scheduler::current_thread::Context::run
   3: my_app::process_request
"#;
        let result = parse_stacktrace(trace);

        assert!(!result.frames[0].is_user_code); // std
        assert!(!result.frames[1].is_user_code); // core
        assert!(!result.frames[2].is_user_code); // tokio
        assert!(result.frames[3].is_user_code); // User code
    }

    // ============= Go Tests =============

    #[test]
    fn test_go_basic_stacktrace() {
        let trace = r#"
goroutine 1 [running]:
main.handler(0x1234)
        /app/handler.go:42 +0x1a
main.main()
        /app/main.go:15 +0x2b
"#;
        let result = parse_stacktrace(trace);

        // Should parse function and location lines
        assert!(result.frames.len() >= 2);
        assert_eq!(result.detected_language, Some(StacktraceLanguage::Go));
    }

    #[test]
    fn test_go_framework_detection() {
        let trace = r#"
runtime.gopanic(0x123)
        /usr/local/go/src/runtime/panic.go:1038 +0x215
net/http.HandlerFunc.ServeHTTP(0x456)
        /usr/local/go/src/net/http/server.go:2012 +0x44
main.myHandler(0x789)
        /app/handler.go:25 +0x1a
"#;
        let result = parse_stacktrace(trace);

        let functions: Vec<_> = result
            .frames
            .iter()
            .filter(|f| f.symbol != "<location>")
            .collect();

        assert!(!functions[0].is_user_code); // runtime
        assert!(!functions[1].is_user_code); // net/http
        assert!(functions[2].is_user_code); // User code
    }

    // ============= Edge Cases =============

    #[test]
    fn test_empty_input() {
        let result = parse_stacktrace("");
        assert!(result.frames.is_empty());
        assert!(result.detected_language.is_none());
    }

    #[test]
    fn test_only_exception_header() {
        let trace = "java.lang.NullPointerException: null";
        let result = parse_stacktrace(trace);
        assert!(result.frames.is_empty());
    }

    #[test]
    fn test_mixed_garbage() {
        let trace = r#"
Some random text
    at com.va.gov.Service.method(Service.java:10)
More random text
"#;
        let result = parse_stacktrace(trace);

        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.unparsed_lines.len(), 2);
    }
}
