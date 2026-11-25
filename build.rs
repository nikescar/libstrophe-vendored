use std::env;
use std::path::PathBuf;

fn main() {
    // Rerun if C source changes
    println!("cargo:rerun-if-changed=c-src");
    println!("cargo:rerun-if-changed=wrapper.h");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Build expat first (libstrophe depends on it)
    build_expat(&out_dir);

    // Build libstrophe
    build_libstrophe(&out_dir);

    // Generate bindings if requested
    #[cfg(feature = "buildtime_bindgen")]
    generate_bindings();
}

fn build_expat(out_dir: &PathBuf) {
    let mut build = cc::Build::new();

    // Core expat source files
    let expat_sources = [
        "c-src/expat/xmlparse.c",
        "c-src/expat/xmlrole.c",
        "c-src/expat/xmltok.c",
    ];

    build
        .files(&expat_sources)
        .include("c-src/expat")
        // expat_config.h is provided in c-src/expat/
        .define("HAVE_EXPAT_CONFIG_H", "1");

    // Platform-specific flags
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "android" | "linux" => {
            build.define("_GNU_SOURCE", None);
            build.define("HAVE_MEMMOVE", "1");
        }
        "windows" => {
            build.define("WIN32", None);
        }
        "macos" | "ios" => {
            build.define("HAVE_MEMMOVE", "1");
        }
        _ => {}
    }

    build.warnings(false); // Suppress C warnings
    build.compile("expat_vendored");

    println!("cargo:lib-dir={}", out_dir.display());
}

fn build_libstrophe(out_dir: &PathBuf) {
    let mut build = cc::Build::new();

    // Core libstrophe source files
    let mut strophe_sources = vec![
        "c-src/libstrophe/auth.c",
        "c-src/libstrophe/compression_dummy.c", // Use dummy (no zlib dependency)
        "c-src/libstrophe/conn.c",
        "c-src/libstrophe/crypto.c",
        "c-src/libstrophe/ctx.c",
        "c-src/libstrophe/deprecated.c",
        "c-src/libstrophe/event.c",
        "c-src/libstrophe/handler.c",
        "c-src/libstrophe/hash.c",
        "c-src/libstrophe/jid.c",
        "c-src/libstrophe/md5.c",
        "c-src/libstrophe/parser_expat.c",
        "c-src/libstrophe/rand.c",
        "c-src/libstrophe/resolver.c",
        "c-src/libstrophe/sasl.c",
        "c-src/libstrophe/scram.c",
        "c-src/libstrophe/sha1.c",
        "c-src/libstrophe/sha256.c",
        "c-src/libstrophe/sha512.c",
        "c-src/libstrophe/snprintf.c",
        "c-src/libstrophe/sock.c",
        "c-src/libstrophe/stanza.c",
        "c-src/libstrophe/tls.c",       // Core TLS abstraction
        "c-src/libstrophe/util.c",
        "c-src/libstrophe/uuid.c",
    ];

    // rustls backend: Rust FFI provides TLS functions, C glue provides accessors
    #[cfg(not(feature = "tls-rustls"))]
    compile_error!("the `tls-rustls` feature is required (it is the only TLS backend)");
    strophe_sources.push("c-src/tls_rustls_glue.c");

    build
        .files(strophe_sources.iter())
        .include("c-src/libstrophe")
        .include("c-src/expat");

    // libstrophe config (hand-rolled, no autoconf)
    build.define("HAVE_EXPAT", "1");

    // Platform-specific configuration
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "android" | "linux" => {
            build.define("_GNU_SOURCE", None);
        }
        "windows" => {
            println!("cargo:rustc-link-lib=ws2_32");
            println!("cargo:rustc-link-lib=crypt32");
        }
        _ => {}
    }

    // C23 (GCC 15+ default) treats `int (*)()` as `int (*)(void)`, breaking handler.c
    build.flag_if_supported("-std=gnu11");
    build.warnings(false);
    build.compile("strophe_vendored");

    println!("cargo:lib-dir={}", out_dir.display());
}

#[cfg(feature = "buildtime_bindgen")]
fn generate_bindings() {
    use bindgen::callbacks::{IntKind, MacroParsingBehavior, ParseCallbacks};

    #[derive(Debug)]
    struct Callbacks;

    impl ParseCallbacks for Callbacks {
        fn will_parse_macro(&self, name: &str) -> MacroParsingBehavior {
            if name.starts_with("XMPP_") {
                MacroParsingBehavior::Default
            } else {
                MacroParsingBehavior::Ignore
            }
        }

        fn int_macro(&self, name: &str, _value: i64) -> Option<IntKind> {
            if name.starts_with("XMPP_E") {
                Some(IntKind::Int)
            } else if name.starts_with("XMPP_CONN_FLAG_") {
                Some(IntKind::Long)
            } else {
                None
            }
        }
    }

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg("-Ic-src/libstrophe")
        .clang_arg("-Ic-src/expat")
        .parse_callbacks(Box::new(Callbacks))
        .blocklist_type("max_align_t")
        .blocklist_type("wchar_t")
        .blocklist_type("__fsid_t")
        .rustified_enum("xmpp_log_level_t")
        .rustified_enum("xmpp_conn_type_t")
        .rustified_enum("xmpp_conn_event_t")
        .rustified_enum("xmpp_error_type_t")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("src/ffi.rs"))
        .expect("Couldn't write bindings");
}
