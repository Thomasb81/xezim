//! SystemC co-simulation CLI-facing configuration.

/// Slice bound (ticks SystemC may run ahead per exchange, SC top). A
/// tuning knob, so it lives in the environment rather than the CLI.
pub fn quantum_from_env() -> u64 {
    std::env::var("XEZIM_SC_QUANTUM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScTop {
    Sv,
    Sc,
}

impl Default for ScTop {
    fn default() -> Self {
        ScTop::Sv
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScDir {
    /// SystemVerilog drives the SystemC signal ($sc_export_sig / `:out`).
    Export,
    /// SystemC drives the SystemVerilog signal ($sc_import_sig / `:in`).
    Import,
    /// Name-matched automatically; synced both ways each exchange. A
    /// signal driven from only one side behaves exactly like the directed
    /// mapping (staging/applying a value that matches the current value is
    /// a no-op); if both sides drive, the SystemC value from the just-
    /// completed slice wins.
    Auto,
}

/// Everything the CLI collects; consumed once at simulation start.
#[derive(Clone, Debug, Default)]
pub struct ScOptions {
    /// The co-simulation master — NEVER user-supplied: computed once from the
    /// sources / prebuilt lib by [`infer_master`]. `--sc-top` was removed by
    /// auto-derivation; see the `sc_main`/`xsc_model_build` symbol analysis in
    /// [`infer_master`].
    pub master: ScTop,
    /// User C++ sources (model for SV top; sc_main TU for SC top).
    pub sources: Vec<String>,
    pub include_dirs: Vec<String>,
    pub cxxflags: Vec<String>,
    /// Prebuilt model .so (built from the user TU + our sc_shim.cc); skips
    /// the compile step.
    pub libs: Vec<String>,
    /// Max ticks the SystemC kernel may run ahead per slice (SC top).
    pub quantum: u64,
    /// systemC module to instantiate for a pure-SV top (`--sc-module <Name>`).
    /// Triggers auto-derived `xsc_model_build` for the SV master direction.
    pub sc_module: Option<String>,
}

static SC_OPTIONS: std::sync::OnceLock<std::sync::Mutex<Option<ScOptions>>> =
    std::sync::OnceLock::new();

pub fn set_sc_options(opts: ScOptions) {
    let cell = SC_OPTIONS.get_or_init(|| std::sync::Mutex::new(None));
    *cell.lock().unwrap() = Some(opts);
}

pub fn sc_options() -> Option<ScOptions> {
    let cell = SC_OPTIONS.get_or_init(|| std::sync::Mutex::new(None));
    cell.lock().unwrap().clone()
}

pub fn sc_top() -> ScTop {
    sc_options().map(|o| o.master).unwrap_or(ScTop::Sv)
}

// Paths of the shim sources, baked by build.rs from the crate manifest dir.
pub fn shim_source() -> &'static str {
    option_env!("XEZIM_SC_SHIM_SRC").unwrap_or("src/sc_shim/sc_shim.cc")
}

pub fn shim_include_dir() -> &'static str {
    option_env!("XEZIM_SC_SHIM_INC").unwrap_or("src/sc_shim")
}

/// Infer which kernel owns the run. This replaces the removed `--sc-top`
/// override: the master is always derived, never user-picked, so the two
/// kernels can never disagree.
///
/// Priority:
/// 1. A prebuilt `--sc-lib` .so **and no `--sc-source`** is probed via its
///    dynamic symbol table: a defined `xsc_model_build` means a passive
///    SystemVerilog top (SV), and a *strong* (defined) `sc_main` means SystemC
///    owns the run (SC); weak (`W`, the shim's fallback) or absent `sc_main`
///    means SV top.
/// 2. Otherwise the `--sc-source` text is scanned comment/string-safely for a
///    *definition* of `xsc_model_build` (SV top, highest priority) or a strong
///    `sc_main` definition (SC top). `--sc-lib` entries alongside sources are
///    extra link libraries and do not affect the master.
pub fn infer_master(sources: &[String], libs: &[String]) -> ScTop {
    // Only probe a prebuilt model when there are no user sources from which
    // to read the entry point; alongside sources a --sc-lib is a link dep.
    if sources.is_empty() {
        if let Some(lib) = libs.iter().find(|l| l.ends_with(".so")) {
            let (t, why) = infer_sc_top_from_so(lib)
                .map(|t| (t, format!("prebuilt {} symbol table", lib)))
                .unwrap_or((
                    ScTop::Sv,
                    format!(
                        "{}: no defined sc_main/xsc_model_build; defaulting to sv",
                        lib
                    ),
                ));
            eprintln!(
                "[xezim-sc] master inferred: {} ({})",
                match t {
                    ScTop::Sc => "sc",
                    ScTop::Sv => "sv",
                },
                why
            );
            return t;
        }
    }
    let (t, why) = infer_sc_top(sources);
    eprintln!(
        "[xezim-sc] master inferred: {} ({})",
        match t {
            ScTop::Sc => "sc",
            ScTop::Sv => "sv",
        },
        why
    );
    t
}

/// Infer which kernel owns the run from the user C++ sources. A TU **defining**
/// `xsc_model_build` is a passive model (SV top); otherwise a TU defining a
/// strong `sc_main` owns the run (SC top). The scan skips comments, strings
/// and character literals (a mention inside those never misfires) and it
/// requires a function *definition* (a `{` body), not a prototype — a
/// header-located `int sc_main(int, char**);` declaration is not a master.
pub fn infer_sc_top(sources: &[String]) -> (ScTop, String) {
    let mut sc_main_file: Option<String> = None;
    for path in sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        match infer_sc_top_from_text(&text) {
            Some(ScTop::Sv) => {
                return (
                    ScTop::Sv,
                    format!("{} defines xsc_model_build (passive model)", path),
                );
            }
            Some(ScTop::Sc) => {
                if sc_main_file.is_none() {
                    sc_main_file = Some(path.clone());
                }
            }
            None => {}
        }
    }
    match sc_main_file {
        Some(p) => (
            ScTop::Sc,
            format!("{} defines a strong sc_main (kernel master)", p),
        ),
        None => (
            ScTop::Sv,
            "no xsc_model_build or strong sc_main found; defaulting to sv".to_string(),
        ),
    }
}

/// Entry-point marker in one source text: `Some(Sv)` for an `xsc_model_build`
/// definition, `Some(Sc)` for a strong `sc_main` definition, `None` for
/// neither. Comments and string/char literals are skipped, and a prototype
/// (`;`, no body) does not count as a definition.
pub fn infer_sc_top_from_text(text: &str) -> Option<ScTop> {
    if fn_is_defined(text, "xsc_model_build") {
        Some(ScTop::Sv)
    } else if fn_is_defined(text, "sc_main") {
        Some(ScTop::Sc)
    } else {
        None
    }
}

/// Does the C++ `text` contain a *definition* (name followed by `(…) {`) of
/// the top-level function `name`? Comments, string literals and char literals
/// are skipped so a mention inside those never matches, and a
/// prototype/declaration (`;` at close-paren, no body) does not count.
fn fn_is_defined(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let nb = name.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Skip comments, string literals, char literals at the outer level so
        // a name inside them is never treated as a candidate.
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len()
                    && !(bytes[i] == b'*' && bytes[i + 1] == b'/')
                {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            _ => {}
        }

        // Candidate: whole-word `name` followed by `(…) {" at function level.
        if bytes[i] == nb[0]
            && i + nb.len() <= bytes.len()
            && &bytes[i..i + nb.len()] == nb
            && (i == 0 || !is_ident_char(bytes[i - 1]))
            && (i + nb.len() >= bytes.len() || !is_ident_char(bytes[i + nb.len()]))
        {
            let after = i + nb.len();
            if function_has_body(&bytes[after..]) {
                return true;
            }
            // Skip past this identifier and keep scanning other candidates.
            i = after;
            continue;
        }
        i += 1;
    }
    false
}

/// Starting just past a candidate function name, scan forward for a `(…)`
/// parameter list followed by a `{` body (definition) as opposed to a `;`
/// (prototype/declaration). Only whitespace, comments and the parameter-list
/// tokens may intervene; anything else (a `;` first, or the body never
/// appearing) means no definition.
fn function_has_body(rest: &[u8]) -> bool {
    let mut j = 0usize;
    let mut depth = 0usize;
    let mut saw_paren = false;
    while j < rest.len() {
        match rest[j] {
            b' ' | b'\t' | b'\n' | b'\r' => j += 1,
            b'/' if j + 1 < rest.len() && rest[j + 1] == b'/' => {
                while j < rest.len() && rest[j] != b'\n' {
                    j += 1;
                }
            }
            b'/' if j + 1 < rest.len() && rest[j + 1] == b'*' => {
                j += 2;
                while j + 1 < rest.len()
                    && !(rest[j] == b'*' && rest[j + 1] == b'/')
                {
                    j += 1;
                }
                j = (j + 2).min(rest.len());
            }
            b'"' => {
                j += 1;
                while j < rest.len() {
                    if rest[j] == b'\\' {
                        j += 2;
                    } else if rest[j] == b'"' {
                        j += 1;
                        break;
                    } else {
                        j += 1;
                    }
                }
            }
            b'\'' => {
                j += 1;
                while j < rest.len() {
                    if rest[j] == b'\\' {
                        j += 2;
                    } else if rest[j] == b'\'' {
                        j += 1;
                        break;
                    } else {
                        j += 1;
                    }
                }
            }
            b';' if !saw_paren => return false,
            b'(' => {
                saw_paren = true;
                depth += 1;
                j += 1;
            }
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 && saw_paren {
                    // Expect `{` (definition) or `;` (prototype).
                    j += 1;
                    while j < rest.len() {
                        match rest[j] {
                            b' ' | b'\t' | b'\n' | b'\r' => j += 1,
                            b'/' if j + 1 < rest.len()
                                && rest[j + 1] == b'/' =>
                            {
                                while j < rest.len() && rest[j] != b'\n' {
                                    j += 1;
                                }
                            }
                            b'/' if j + 1 < rest.len()
                                && rest[j + 1] == b'*' =>
                            {
                                j += 2;
                                while j + 1 < rest.len()
                                    && !(rest[j] == b'*' && rest[j + 1] == b'/')
                                {
                                    j += 1;
                                }
                                j = (j + 2).min(rest.len());
                            }
                            b'{' => return true,
                            b';' => return false,
                            _ => j += 1,
                        }
                    }
                    return false;
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    false
}

/// Probe a prebuilt SystemC/co-simulation `.so` for its entry-point symbol
/// and infer the master. `Some(Sc)` when `sc_main` is a *strong* (defined)
/// dynamic symbol; `Some(Sv)` when `xsc_model_build` is defined; `None` when
/// the file can't be read or neither is present (caller defaults to SV). Uses
/// the dynamic symbol table (`nm -D`), so the shim's *weak* `sc_main`
/// fallback in an SV-top model is correctly NOT treated as SC top.
fn infer_sc_top_from_so(path: &str) -> Option<ScTop> {
    if has_so_symbol_of(path, "xsc_model_build") {
        return Some(ScTop::Sv);
    }
    if has_strong_so_sc_main(path) {
        return Some(ScTop::Sc);
    }
    None
}

/// True when `nm -D` reports `name` as a *defined* dynamic symbol (any
/// strength, local or global). An undefined (`U`) reference does not count.
/// The match is a substring test so `extern "C"` (`xsc_model_build`) and C++
/// mangled forms (`_Z7sc_mainiPPc`) both hit.
fn has_so_symbol_of(path: &str, name: &str) -> bool {
    so_dynsym_lines(path)
        .into_iter()
        .any(|(sym, t)| sym.contains(name) && t != b'U')
}

/// True when `nm -D` reports a *strong* definition of `sc_main` (type `T` or
/// `D`); weak (`W`) or undefined (`U`) means the shim's fallback and is
/// rejected. A C++ model's `sc_main` mangles (`_Z7sc_mainiPPc`), so the test
/// is a substring match on the demangled name.
fn has_strong_so_sc_main(path: &str) -> bool {
    so_dynsym_lines(path)
        .into_iter()
        .any(|(sym, t)| sym.contains("sc_main") && (t == b'T' || t == b'D'))
}

/// Run `nm -D` against a prebuilt `.so` and return `(symbol, type_char)` for
/// every dynamic symbol. Tries POSIX mode first, classic mode second. Never
/// panics on a build/host quirk (returns empty on any failure).
fn so_dynsym_lines(path: &str) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    for posix in [true, false] {
        let cmd: Vec<String> = {
            let mut c = vec!["nm".to_string(), "-D".to_string()];
            if posix {
                c.push("--format=posix".to_string());
            }
            c.push(path.to_string());
            c
        };
        let Ok(res) = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .env("LC_ALL", "C")
            .output()
        else {
            continue;
        };
        if !res.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&res.stdout);
        let parsed: Vec<(String, u8)> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                // POSIX: `name type value`; classic: `value type name`.
                let mut it = line.split_whitespace();
                let a = it.next()?;
                let b = it.next()?;
                let c = it.next()?;
                if posix {
                    // `name` is the first token; `type` the second (single
                    // char, may be multi-char like Td/Tt in some nm builds).
                    let t = b.as_bytes().first().copied()?;
                    Some((a.to_string(), t))
                } else {
                    // classic: address is first, type second, name third.
                    let t = b.as_bytes().first().copied()?;
                    Some((c.to_string(), t))
                }
            })
            .collect();
        out = parsed;
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// A port discovered on a SystemC module from its C++ source text.
/// `name` is the C++ member identifier, `dir` the SystemC direction,
/// `width` the 2-state width derived from the template argument, and
/// `align` the byte-alignment SystemC uses for that width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScPort {
    pub name: String,
    pub dir: ScDir,
    pub width: u32,
    pub align: u32,
    /// Normalized SystemC template type (`bool`, `sc_uint<8>`, `int`, …)
    /// with `sc_core::`/`sc_dt::`/`std::` prefixes stripped. Used to type the
    /// backing `sc_core::sc_signal<T>` the leaf model binds to the port.
    pub sc_type: String,
}

/// Textually reflect the interface of `SC_MODULE(<sc_module>)` from the user
/// C++ sources (`--sc-source`). Used by the SystemVerilog-master (Mode B)
/// path so the elaborator can treat the SystemC module as a leaf foreign
/// instance: its `sc_in`/`sc_out`/`sc_inout` member declarations become the
/// instance's ports before the SV netlist connects them.
///
/// Only a subset of template types (the ones our crossing supports) map to a
/// concrete width: `bool` -> 1, `uint64`/uint64_t and `uintN`/`sc_uint<N>`
/// -> N, `sc_bv<N>`/`sc_lv<N>` -> N, and `int` -> 32. Anything else is
/// skipped (the SV side errors on an unresolved port reference rather than
/// guess). Unqualified `sc_in` etc. (no `sc_core::`) are matched too, and
/// either spelling is accepted. Internal `sc_signal` members and plain data
/// members are NOT ports and are not reflected.
///
/// This mirrors how the codebase already sniffs entry-points by text
/// (`infer_sc_top_from_text`); it is deliberately conservative and is only a
/// *hint* consumed when real Mode B is enabled.
pub fn sc_module_ports(sc_module: &str, sources: &[String]) -> Vec<ScPort> {
    let mut out = Vec::new();
    // Find the SC_MODULE(<name>) block start in any source file. A module may
    // span multiple occurrences (instantiation !== declaration), and the
    // declaration is the one whose body lists ports — but simply scanning the
    // first block headed by SC_MODULE(<name>) is enough for our generated
    // examples.
    for path in sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        // Locate the declaration: `SC_MODULE(<name>) {` ... matching `};` at
        // column 0 (the SystemC macro convention).
        let needle = format!("SC_MODULE({})", sc_module);
        let Some(hit) = text.find(&needle) else {
            continue;
        };
        let rest = &text[hit + needle.len()..];
        let mut depth = 0usize;
        let mut in_string = false;
        let mut in_block = false;
        let mut body_end = rest.len() - 1; // safety default
        for (i, ch) in rest.char_indices() {
            match ch {
                '"' if !in_string && (i == 0 || rest[..i].chars().last() != Some('\\')) => {
                    in_string = true;
                }
                '"' if in_string => in_string = false,
                _ if !in_string => match ch {
                    '{' => {
                        if !in_block {
                            in_block = true;
                            depth = 1;
                        } else {
                            depth += 1;
                        }
                    }
                    '}' => {
                        if in_block {
                            depth -= 1;
                            if depth == 0 {
                                body_end = i;
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let body = &rest[..body_end];
        for port in scan_sc_port_members(body) {
            out.push(port);
        }
        break; // only the first matching source file
    }
    out
}

/// Scan the SystemVerilog sources for direct instantiations of the SystemC
/// module `<sc_module>` and return the instance identifiers in source order.
///
/// This is the SV-top-mode chicken-and-egg breaker for *multiple* netlist
/// SC leaves: instance names are needed to scope the generated C++ wrapper's
/// backing signals and registry names before elaboration. Matches the syntax
/// `inc_leaf ua (.port(net), ...);` / `inc_leaf ua #(.p(v)) (.port(net));`.
pub fn sc_instances_in_sv(sc_module: &str, sources: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let needle = sc_module.to_string();
    for path in sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let bytes = text.as_bytes();
        let mut i = 0usize;
        'outer: while i + needle.len() <= bytes.len() {
            // Find the module identifier as a whole word (`needle` preceded
            // by a non-identifier char).
            if !text[i..].starts_with(&needle) {
                i += 1;
                continue;
            }
            if i > 0 && is_ident_char(bytes[i - 1]) {
                i += 1;
                continue;
            }
            let after = i + needle.len();
            if after < bytes.len() && is_ident_char(bytes[after]) {
                i += 1;
                continue;
            }
            i = after;
            // Skip whitespace / comments to the instance identifier.
            let mut j = i;
            let mut inst_name = String::new();
            let mut found_inst = false;
            while j < bytes.len() {
                match bytes[j] {
                    b' ' | b'\t' | b'\n' | b'\r' => j += 1,
                    b'/' if j + 1 < bytes.len() && bytes[j + 1] == b'/' => {
                        // line comment
                        while j < bytes.len() && bytes[j] != b'\n' {
                            j += 1;
                        }
                    }
                    b'/' if j + 1 < bytes.len() && bytes[j + 1] == b'*' => {
                        j += 2;
                        while j + 1 < bytes.len()
                            && !(bytes[j] == b'*' && bytes[j + 1] == b'/')
                        {
                            j += 1;
                        }
                        j = (j + 2).min(bytes.len());
                    }
                    c if is_ident_char(c) => {
                        let start = j;
                        while j < bytes.len() && is_ident_char(bytes[j]) {
                            j += 1;
                        }
                        inst_name = text[start..j].to_string();
                        found_inst = true;
                        break;
                    }
                    _ => break 'outer,
                }
            }
            if !found_inst {
                continue 'outer;
            }
            // Skip whitespace + optional `#(...)` parameter list, then expect
            // the port-connection `(`. If not, this wasn't an instantiation.
            let mut k = j;
            let mut is_inst = false;
            while k < bytes.len() {
                match bytes[k] {
                    b' ' | b'\t' | b'\n' | b'\r' => k += 1,
                    b'#' => {
                        // skip the #( ... ) balanced parens
                        k += 1;
                        while k < bytes.len() && bytes[k] != b'(' {
                            k += 1;
                        }
                        let mut depth = 0usize;
                        while k < bytes.len() {
                            match bytes[k] {
                                b'(' => depth += 1,
                                b')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        k += 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            k += 1;
                        }
                    }
                    b'(' => {
                        is_inst = true;
                        break;
                    }
                    b';' => break,
                    _ => break 'outer,
                }
            }
            if is_inst {
                out.push(inst_name);
            }
            i = k;
        }
    }
    out
}

/// Extract `sc_in<...>`, `sc_out<...>`, `sc_inout<...>` member declarations
/// (the crossing ports) from a SystemC module body text.
///
/// Hand-rolled rather than regex: the template argument may contain nested
/// `<...>` (e.g. `sc_uint<8>`), so we scan keyword occurrences and
/// walk balanced angle-brackets to the matching `>`, then read the member
/// identifier up to the next `;`.
fn scan_sc_port_members(body: &str) -> Vec<ScPort> {
    let mut out = Vec::new();
    let keywords = [
        ("sc_in", ScDir::Import),
        ("sc_out", ScDir::Export),
        ("sc_inout", ScDir::Auto), // genuinely bidirectional
    ];
    // Deliberately NOT `sc_signal`: those members are internal/deep signals
    // (e.g. hierarchical SC modules wire children with sc_signal interconns),
    // never module ports. Only sc_in/sc_out/sc_inout are the crossing
    // boundary. Reflecting sc_signal as a port broke the reference
    // `vlog_sc` ringbuf (its internal ramadrs/buffer/oeenable signals were
    // mis-bound as ports).
    let bytes = body.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        // Skip strings and comments so identifiers inside them are ignored.
        match bytes[i] {
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
            b'"' => {
                i += 1;
                while i < n && bytes[i] != b'"' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        // Match a keyword with a word/id boundary before and after.
        let mut matched: Option<(usize, ScDir)> = None;
        for (kw, dir) in &keywords {
            if body[i..].starts_with(kw) {
                let after = i + kw.len();
                let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
                let after_ok = after >= n || !is_ident_char(bytes[after]);
                if before_ok && after_ok {
                    matched = Some((after, *dir));
                    break;
                }
            }
        }
        let Some((after_kw, dir)) = matched else {
            i += 1;
            continue;
        };
        // Optional `sc_core::` scope already consumed by the ident-boundary
        // check; the keyword itself may be preceded by `::`. Then an optional
        // template `<...>`.
        let mut j = after_kw;
        // skip spaces
        while j < n && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\r')
        {
            j += 1;
        }
        let mut ty = String::new();
        if j < n && bytes[j] == b'<' {
            let mut depth = 0usize;
            let start = j;
            while j < n {
                match bytes[j] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            ty = body[start + 1..j - 1].replace(' ', "").to_string();
        }
        // Member declarator: (name | array | (func-args))? ; — take the first
        // identifier after the template as the port name.
        let mut k = j;
        while k < n && (bytes[k].is_ascii_whitespace() || bytes[k] == b',' || bytes[k] == b';')
        {
            if bytes[k] == b';' {
                break;
            }
            k += 1;
        }
        // read ident
        let name_start = k;
        while k < n && is_ident_char(bytes[k]) {
            k += 1;
        }
        let name = body[name_start..k].to_string();
        if name.is_empty() {
            i = j.max(i + 1);
            continue;
        }
        if let Some((width, align)) = sc_type_width(&ty, &name) {
            out.push(ScPort {
                name,
                dir,
                width,
                align,
                sc_type: normalize_sc_type(&ty),
            });
        }
        i = k.max(i + 1);
    }
    out
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Strip `sc_core::`/`sc_dt::`/`std::` prefixes (all spellings appear) from a
/// SystemC template type, yielding the canonical `bool` / `sc_uint<8>` / `int`
/// etc. used both for width inference and to type the bridge backing signal.
fn normalize_sc_type(ty: &str) -> String {
    let mut t = ty.trim().to_string();
    loop {
        let before = t.clone();
        for prefix in ["sc_core::", "sc_dt::", "std::"] {
            if let Some(rest) = t.trim().strip_prefix(prefix) {
                t = rest.trim().to_string();
            }
        }
        if t == before {
            break;
        }
    }
    t
}

/// Map a SystemC port template type to a (2-state width, byte alignment).
/// `None` means "unhandled" (skipped, conservative).
fn sc_type_width(ty: &str, _name: &str) -> Option<(u32, u32)> {
    // Strip sc_core:: / sc_dt:: and std:: prefixes (all spellings appear).
    let mut core_free = ty.trim();
    for prefix in ["sc_core::", "sc_dt::", "std::"] {
        if let Some(rest) = core_free.strip_prefix(prefix) {
            core_free = rest.trim();
        }
    }
    match core_free {
        "bool" => Some((1, 1)),
        "uint64" | "uint64_t" => Some((64, 8)),
        "int" => Some((32, 4)),
        _ => {
            // sc_uint<N>, sc_bv<N>, sc_lv<N>, sc_reg<N>
            if let Some(d) = angle_num(core_free.strip_prefix("sc_uint")) {
                let align = ((d + 7) / 8).max(1);
                return Some((d, align));
            }
            let pfx = ["sc_bv", "sc_lv", "sc_reg"]
                .into_iter()
                .find(|p| core_free.starts_with(p));
            if let Some(p) = pfx {
                if let Some(d) = angle_num(core_free.strip_prefix(p)) {
                    let align = ((d + 7) / 8).max(1);
                    return Some((d, align));
                }
            }
            None
        }
    }
}

/// Parse `N` from a string that may begin `<N>` (an angle-bracket integer).
fn angle_num(mut s: Option<&str>) -> Option<u32> {
    s = s.map(str::trim);
    let inner = s?.strip_prefix("<")?;
    let end = inner.find('>')?;
    inner[..end].trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sc_top_inference_prefers_model_build() {
        // xsc_model_build wins even when both entry points are mentioned.
        assert_eq!(
            infer_sc_top_from_text(
                "int sc_main(int, char**) { return 0; }\nvoid* xsc_model_build() { return nullptr; }\n",
            ),
            Some(ScTop::Sv)
        );
    }

    #[test]
    fn sc_top_inference_finds_sc_main() {
        assert_eq!(
            infer_sc_top_from_text(
                "int sc_main(int argc, char* argv[]) { return 0; }"
            ),
            Some(ScTop::Sc)
        );
    }

    #[test]
    fn sc_top_inference_defaults_to_sv() {
        assert_eq!(infer_sc_top_from_text("// just a comment\n"), None);
    }

    #[test]
    fn sc_top_inference_prototype_is_not_a_definition() {
        // A header-style declaration (no body) is NOT a master.
        assert_eq!(
            infer_sc_top_from_text("int sc_main(int argc, char** argv);"),
            None
        );
    }

    #[test]
    fn sc_top_inference_ignores_comments_and_strings() {
        // Mentions inside comments and string literals must not misfire.
        assert_eq!(
            infer_sc_top_from_text("// sc_main(int,char**) would go here\n"),
            None
        );
        assert_eq!(
            infer_sc_top_from_text(
                "const char* s = \"sc_main\"; /* xsc_model_build () {} */"
            ),
            None
        );
        assert_eq!(
            infer_sc_top_from_text("int sc_main(int,char**) { return 0; }"),
            Some(ScTop::Sc)
        );
    }

    #[test]
    fn sc_top_inference_sc_main_member_is_not_the_master() {
        // `module.sc_main(...)` is a member call, not the global sc_main
        // entry point; the scanner only matches a whole identifier and still
        // requires a `{` body at function level.
        assert_eq!(
            infer_sc_top_from_text("void f() { foo.sc_main(x); }"),
            None
        );
    }

    #[test]
    fn sc_prebuilt_so_symbol_probe() {
        use std::process::Command;
        // Build a small .so exporting a strong sc_main and check the probe,
        // then a second exporting only a weak-style xsc_model_build.
        let dir = std::env::temp_dir();
        let a = dir.join("xez_sc_probe_a.cc");
        let b = dir.join("xez_sc_probe_b.cc");
        std::fs::write(&a, "int sc_main(int, char**) { return 0; }\n").unwrap();
        std::fs::write(&b, "extern \"C\" void xsc_model_build() {}\n").unwrap();
        let so_a = dir.join("libxez_sc_a.so");
        let so_b = dir.join("libxez_sc_b.so");
        let _ = std::fs::remove_file(&so_a);
        let _ = std::fs::remove_file(&so_b);
        let gxx = |src: &std::path::Path, out: &std::path::Path| {
            Command::new("g++")
                .arg("-shared")
                .arg("-fPIC")
                .arg("-o")
                .arg(out)
                .arg(src)
                .status()
        };
        if gxx(&a, &so_a).map(|s| s.success()).unwrap_or(false)
            && gxx(&b, &so_b).map(|s| s.success()).unwrap_or(false)
        {
            assert_eq!(infer_sc_top_from_so(so_a.to_str().unwrap()), Some(ScTop::Sc));
            assert_eq!(infer_sc_top_from_so(so_b.to_str().unwrap()), Some(ScTop::Sv));
        } else {
            eprintln!("g++ unavailable; skipping .so symbol probe test");
        }
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
        let _ = std::fs::remove_file(so_a);
        let _ = std::fs::remove_file(so_b);
    }

    #[test]
    fn sc_module_port_reflection_sc_uint() {
        let src = r#"
#include <systemc>
SC_MODULE(ringbuf) {
  sc_in<bool> clock;
  sc_in<bool> reset;
  sc_in<sc_uint<8>> data_in;
  sc_out<sc_uint<8>> data_out;
  sc_core::sc_inout<sc_core::sc_uint<4>> nibble;
  SC_CTOR(ringbuf) {}
};
"#;
        let dir = std::env::temp_dir();
        let p = dir.join("scmod_abc.cc");
        std::fs::write(&p, src).unwrap();
        let ports = sc_module_ports("ringbuf", &[p.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_file(&p);
        // Order: clock, reset, data_in, data_out, nibble.
        assert_eq!(ports.len(), 5);
        assert_eq!(ports[0].name, "clock");
        assert_eq!(ports[0].dir, ScDir::Import);
        assert_eq!(ports[0].width, 1);
        assert_eq!(ports[1].name, "reset");
        assert_eq!(ports[1].width, 1);
        assert_eq!(ports[2].name, "data_in");
        assert_eq!(ports[2].width, 8);
        assert_eq!(ports[3].name, "data_out");
        assert_eq!(ports[3].width, 8);
        assert_eq!(ports[3].dir, ScDir::Export);
        assert_eq!(ports[4].name, "nibble");
        assert_eq!(ports[4].width, 4);
        assert_eq!(ports[4].dir, ScDir::Auto);
    }

    #[test]
    fn sc_module_port_reflection_uint64_and_bv() {
        let src = r#"
#include <systemc>
SC_MODULE(m) {
  sc_core::sc_in<std::uint64_t> a;
  sc_core::sc_out<std::uint64_t> b;
  sc_in<sc_bv<16>> w;
  sc_signal<bool> internal;   // internal only — NOT a port, must be excluded
  SC_CTOR(m) {}
};
"#;
        let dir = std::env::temp_dir();
        let p = dir.join("scmod_def.cc");
        std::fs::write(&p, src).unwrap();
        let ports = sc_module_ports("m", &[p.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_file(&p);
        assert_eq!(ports.len(), 3); // sc_signal<bool> internal is not a port
        assert_eq!(ports[0].name, "a");
        assert_eq!(ports[0].width, 64);
        assert_eq!(ports[0].dir, ScDir::Import);
        assert_eq!(ports[1].name, "b");
        assert_eq!(ports[1].width, 64);
        assert_eq!(ports[2].name, "w");
        assert_eq!(ports[2].width, 16);
    }

    #[test]
    fn sc_module_port_reflection_ignores_keyword_in_comment_string() {
        let src = r#"
// sc_in<bool> not_a_port;
const char* note = "sc_out note";
SC_MODULE(m2) {
  sc_in<bool> real_port;
  SC_CTOR(m2) {}
};
"#;
        let dir = std::env::temp_dir();
        let p = dir.join("scmod_ghi.cc");
        std::fs::write(&p, src).unwrap();
        let ports = sc_module_ports("m2", &[p.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_file(&p);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "real_port");
    }
}