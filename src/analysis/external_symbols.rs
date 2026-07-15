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
pub fn is_python_stdlib_module(dotted_name: &str) -> bool {
    let top_level = dotted_name.split('.').next().unwrap_or(dotted_name);
    PYTHON_STDLIB_MODULES.contains(&top_level)
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
pub fn normalize_python_package_name(name: &str) -> String {
    name.chars()
        .map(|character| match character {
            '-' | '.' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Rust standard-library crate names that ship with every toolchain.
const RUST_STD_CRATES: &[&str] = &["std", "core", "alloc"];

/// Common prelude/std types and traits referenced by bare name (e.g. in a
/// trait-impl target like `impl Debug for Foo`), where the analyzer never
/// sees a `std::`-prefixed path to classify by prefix.
const RUST_PRELUDE_TYPES: &[&str] = &[
    "Vec",
    "Option",
    "Result",
    "Box",
    "String",
    "str",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "VecDeque",
    "Rc",
    "Arc",
    "RefCell",
    "Cell",
    "Cow",
    "Iterator",
    "IntoIterator",
    "Debug",
    "Display",
    "Clone",
    "Copy",
    "Default",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Send",
    "Sync",
    "Drop",
    "From",
    "Into",
    "TryFrom",
    "TryInto",
    "AsRef",
    "AsMut",
    "Deref",
    "DerefMut",
    "Fn",
    "FnMut",
    "FnOnce",
    "Error",
];

/// The std crate a `use` path (with any leading `crate::` already stripped)
/// is rooted at (`std`/`core`/`alloc`), or `None` for anything else; used to
/// collapse every std-prefixed `use` into one shared external package node
/// per crate rather than one per path.
pub fn rust_std_crate(path: &str) -> Option<&'static str> {
    let path = path.strip_prefix("::").unwrap_or(path);
    let top_level = path.split("::").next().unwrap_or(path);
    RUST_STD_CRATES
        .iter()
        .find(|crate_name| **crate_name == top_level)
        .copied()
}

/// True when `name` is a well-known prelude type/trait referenced by bare
/// name (no path prefix for [`rust_std_crate`] to classify).
pub fn is_rust_prelude_type(name: &str) -> bool {
    RUST_PRELUDE_TYPES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{
        is_python_stdlib_module, is_rust_prelude_type, normalize_python_package_name,
        rust_std_crate,
    };

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

    #[test]
    fn recognizes_rust_prelude_types_by_exact_name() {
        assert!(is_rust_prelude_type("Vec"));
        assert!(is_rust_prelude_type("Debug"));
        assert!(!is_rust_prelude_type("MyStruct"));
    }
}
