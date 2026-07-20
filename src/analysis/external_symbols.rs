//! Classification of references that are known to point at a language's
//! standard library or prelude rather than at repository code.
//!
//! Without this, every `import os` or `use std::collections::HashMap` becomes
//! an `Unresolved` graph node indistinguishable from a genuine unresolved
//! intra-repo reference, and every file in the repo re-triggers the same
//! stdlib noise. Classifying these separately (as an external `Package`
//! node, deduplicated by name) keeps `Unresolved` meaningful.

/// Python 3 standard library top-level module names (`sys.stdlib_module_names`
/// as of CPython 3.11, minus deprecated/internal `_`-prefixed entries that
/// never appear in application import statements). Bundled as a static list
/// rather than introspected at runtime, since Lithograph never shells out to
/// a Python interpreter.
const PYTHON_STDLIB_MODULES: &[&str] = &[
    "__future__",
    "abc",
    "aifc",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "atexit",
    "base64",
    "bisect",
    "builtins",
    "bz2",
    "calendar",
    "cgi",
    "cgitb",
    "chunk",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "colorsys",
    "compileall",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "cProfile",
    "csv",
    "ctypes",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "doctest",
    "email",
    "encodings",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "imaplib",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "lib2to3",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "mailcap",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "multiprocessing",
    "netrc",
    "nntplib",
    "numbers",
    "operator",
    "optparse",
    "os",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pipes",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posixpath",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtpd",
    "smtplib",
    "sndhdr",
    "socket",
    "socketserver",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sunau",
    "symtable",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "telnetlib",
    "tempfile",
    "termios",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "turtledemo",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uu",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "wsgiref",
    "xdrlib",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

/// True when `dotted_name` (e.g. `"os"`, `"os.path"`, `"collections.abc"`)
/// refers to the Python standard library, judged by its top-level segment.
pub(crate) fn is_python_stdlib_module(dotted_name: &str) -> bool {
    let top_level = dotted_name.split('.').next().unwrap_or(dotted_name);
    PYTHON_STDLIB_MODULES.contains(&top_level)
}

/// Python `builtins`-module names a source file uses without importing them --
/// the Python counterpart of [`is_javascript_builtin`] (LIT-77) and
/// [`is_python_stdlib_module`] (LIT-6). A curated list of the builtin types,
/// functions, exceptions, and constants only: a bare `str(x)`, `isinstance(...)`,
/// `ValueError(...)`, or `x: bool` names a builtin, not a missing project
/// symbol, so classifying it stops it diluting the genuine-gap Unresolved
/// bucket. Deliberately excludes dunder attributes and `typing` members
/// (`Self`, `Optional`) -- those come from imports and are resolved through
/// them, not guessed here.
const PYTHON_BUILTINS: &[&str] = &[
    // Types and constructors.
    "bool",
    "bytearray",
    "bytes",
    "complex",
    "dict",
    "float",
    "frozenset",
    "int",
    "list",
    "memoryview",
    "object",
    "range",
    "set",
    "slice",
    "str",
    "tuple",
    "type",
    // Functions.
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "breakpoint",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "delattr",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "format",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "locals",
    "map",
    "max",
    "min",
    "next",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "repr",
    "reversed",
    "round",
    "setattr",
    "sorted",
    "staticmethod",
    "sum",
    "super",
    "vars",
    "zip",
    // Exceptions and warnings.
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    // Constants.
    "Ellipsis",
    "False",
    "None",
    "NotImplemented",
    "True",
];

/// True when `name` is a bare Python builtin (type, function, exception, or
/// constant) usable without an import. Exact-match only: a dotted or member
/// name is not a bare global, and a user symbol that merely shares the spelling
/// is not shadowed here -- shadowing is judged by the caller against local
/// definitions, exactly as [`is_javascript_builtin`] is used.
pub(crate) fn is_python_builtin(name: &str) -> bool {
    PYTHON_BUILTINS.contains(&name)
}

/// Normalizes a Python package name for comparing a manifest-declared
/// dependency name (e.g. `python-dateutil`, PEP 503 style) against a source
/// file's import module name (`dateutil`) or vice versa: lowercased, with
/// `-` and `.` folded to `_`. This only closes the hyphen/case/separator gap
/// between how a name is written in `pyproject.toml`/`requirements.txt`
/// versus a Python `import` statement -- it deliberately does not attempt to
/// bridge distribution names that differ entirely from their import name
/// (`PyYAML` imports as `yaml`, `beautifulsoup4` as `bs4`, `Pillow` as `PIL`).
/// Those stay `Unresolved`, same as any other unmatched reference; a curated
/// alias table would need ongoing upkeep this crate deliberately avoids.
pub(crate) fn normalize_python_package_name(name: &str) -> String {
    name.chars()
        .map(|character| match character {
            '-' | '.' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Rust standard-library crate names that ship with every toolchain.
const RUST_STD_CRATES: &[&str] = &["std", "core", "alloc"];

/// The std crate a `use` path (with any leading `crate::` already stripped)
/// is rooted at (`std`/`core`/`alloc`), or `None` for anything else; used to
/// collapse every std-prefixed `use` into one shared external package node
/// per crate rather than one per path.
pub(crate) fn rust_std_crate(path: &str) -> Option<&'static str> {
    let path = path.strip_prefix("::").unwrap_or(path);
    let top_level = path.split("::").next().unwrap_or(path);
    RUST_STD_CRATES
        .iter()
        .find(|crate_name| **crate_name == top_level)
        .copied()
}

/// ECMAScript, DOM/BOM, Node, and TypeScript-library global names that a
/// TS/JS file references by bare name without importing them -- the JS
/// counterpart of [`is_python_stdlib_module`] (LIT-6) and
/// bare-name Rust prelude types (LIT-66). Deliberately a curated list of *global*
/// identifiers and standard library types only. Prototype method names
/// (`forEach`, `map`, `filter`) are excluded on purpose: they are receiver
/// members, not globals, so a bare `filter` could equally be a lodash import
/// or a local method, and classifying it as a builtin would be a guess. Those
/// stay `Unresolved`, which is the honest answer.
const JAVASCRIPT_BUILTINS: &[&str] = &[
    // ECMAScript globals and constructors.
    "Array",
    "ArrayBuffer",
    "BigInt",
    "BigInt64Array",
    "BigUint64Array",
    "Boolean",
    "DataView",
    "Date",
    "Error",
    "EvalError",
    "FinalizationRegistry",
    "Float32Array",
    "Float64Array",
    "Function",
    "Infinity",
    "Int8Array",
    "Int16Array",
    "Int32Array",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URIError",
    "Uint8Array",
    "Uint8ClampedArray",
    "Uint16Array",
    "Uint32Array",
    "WeakMap",
    "WeakRef",
    "WeakSet",
    "globalThis",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "eval",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
    "structuredClone",
    "undefined",
    // DOM/BOM globals and interfaces.
    "AbortController",
    "AbortSignal",
    "Blob",
    "CustomEvent",
    "Document",
    "DocumentFragment",
    "Element",
    "Event",
    "EventTarget",
    "File",
    "FileReader",
    "FormData",
    "Headers",
    "HTMLAnchorElement",
    "HTMLButtonElement",
    "HTMLCanvasElement",
    "HTMLDivElement",
    "HTMLElement",
    "HTMLFormElement",
    "HTMLImageElement",
    "HTMLInputElement",
    "HTMLSelectElement",
    "HTMLTextAreaElement",
    "IntersectionObserver",
    "MutationObserver",
    "Node",
    "Request",
    "Response",
    "ResizeObserver",
    "URL",
    "URLSearchParams",
    "WebSocket",
    "Window",
    "Worker",
    "XMLHttpRequest",
    "alert",
    "atob",
    "btoa",
    "clearInterval",
    "clearTimeout",
    "confirm",
    "console",
    "document",
    "fetch",
    "history",
    "localStorage",
    "location",
    "navigator",
    "queueMicrotask",
    "requestAnimationFrame",
    "sessionStorage",
    "setInterval",
    "setTimeout",
    "window",
    // Node globals.
    "Buffer",
    "__dirname",
    "__filename",
    "global",
    "process",
    // TypeScript standard-library utility types.
    "Awaited",
    "Capitalize",
    "Exclude",
    "Extract",
    "InstanceType",
    "Lowercase",
    "NonNullable",
    "Omit",
    "Parameters",
    "Partial",
    "Pick",
    "Readonly",
    "Record",
    "Required",
    "ReturnType",
    "Uppercase",
];

/// True when `name` is a bare global identifier or standard-library type built
/// into JavaScript/TypeScript runtimes, so a reference to it is external
/// rather than a missing local symbol. Exact-match only: these are unqualified
/// global names, not dotted paths.
pub(crate) fn is_javascript_builtin(name: &str) -> bool {
    JAVASCRIPT_BUILTINS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{
        is_javascript_builtin, is_python_builtin, is_python_stdlib_module,
        normalize_python_package_name, rust_std_crate,
    };

    #[test]
    fn recognizes_javascript_builtins_by_exact_bare_name() {
        assert!(is_javascript_builtin("Array"));
        assert!(is_javascript_builtin("JSON"));
        assert!(is_javascript_builtin("btoa"));
        assert!(is_javascript_builtin("Record"));
        assert!(is_javascript_builtin("HTMLButtonElement"));
        // A dotted path is not a bare global; user types are not builtins.
        assert!(!is_javascript_builtin("Array.prototype"));
        assert!(!is_javascript_builtin("ApiError"));
        // Prototype method names are deliberately excluded (receiver members,
        // not globals): a bare `filter` could be a local or an import.
        assert!(!is_javascript_builtin("filter"));
        assert!(!is_javascript_builtin("forEach"));
    }

    #[test]
    fn recognizes_python_builtins_by_exact_bare_name() {
        assert!(is_python_builtin("str"));
        assert!(is_python_builtin("bool"));
        assert!(is_python_builtin("isinstance"));
        assert!(is_python_builtin("ValueError"));
        assert!(is_python_builtin("None"));
        // A user model shares no spelling with a builtin here, and a dotted or
        // typing name is not a bare builtin.
        assert!(!is_python_builtin("Message"));
        assert!(!is_python_builtin("Self"));
        assert!(!is_python_builtin("str.join"));
    }

    #[test]
    fn recognizes_python_stdlib_modules_by_top_level_segment() {
        assert!(is_python_stdlib_module("os"));
        assert!(is_python_stdlib_module("os.path"));
        assert!(is_python_stdlib_module("collections.abc"));
        assert!(!is_python_stdlib_module("requests"));
        assert!(!is_python_stdlib_module("my_package.os"));
    }

    #[test]
    fn normalizes_package_names_for_manifest_vs_import_comparison() {
        assert_eq!(normalize_python_package_name("fastapi"), "fastapi");
        assert_eq!(normalize_python_package_name("FastAPI"), "fastapi");
        assert_eq!(
            normalize_python_package_name("python-dateutil"),
            "python_dateutil"
        );
        assert_eq!(
            normalize_python_package_name("python_dateutil"),
            "python_dateutil"
        );
    }

    #[test]
    fn recognizes_rust_std_paths_by_top_level_crate() {
        assert_eq!(rust_std_crate("std::collections::HashMap"), Some("std"));
        assert_eq!(rust_std_crate("core::fmt::Debug"), Some("core"));
        assert_eq!(rust_std_crate("alloc::vec::Vec"), Some("alloc"));
        assert_eq!(rust_std_crate("std::env::var"), Some("std"));
        assert_eq!(rust_std_crate("serde::Serialize"), None);
        assert_eq!(rust_std_crate("stdlib_lookalike::Thing"), None);
    }
}
