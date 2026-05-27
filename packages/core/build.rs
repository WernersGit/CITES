use std::env;
use std::path::PathBuf;
use std::fs;

/// Returns extra clang args needed when cross-compiling for the iOS Simulator
///
/// `bindgen` passes the Rust target triple directly to libclang, but Clang
/// rejects `arm64-apple-ios-sim` becasue it has no version component:
/// Replacing it with the fully-qualified form and add the SDK sysroot so that
/// system headers are found.
fn ios_sim_clang_args() -> Vec<String> {
    let sdk = std::process::Command::new("xcrun")
        .args(["--show-sdk-path", "--sdk", "iphonesimulator"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    let Some(sysroot) = sdk else { return vec![]; };

    let min_version = env::var("IPHONEOS_DEPLOYMENT_TARGET")
        .unwrap_or_else(|_| "16.0".to_string());

    vec![
        format!("--sysroot={sysroot}"),
        "-target".to_string(),
        format!("arm64-apple-ios{min_version}-simulator"),
    ]
}

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set");
    let asn1_c_dir = PathBuf::from(manifest_dir).join("src/asn1_c");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    if !asn1_c_dir.exists() {
        println!("cargo:warning=Directory {} not found. Run build_asn1_c.sh first.", asn1_c_dir.display());
        return;
    }

    println!("cargo:rerun-if-changed={}", asn1_c_dir.display());

    let target = env::var("TARGET").unwrap_or_default();
    let extra_clang_args: Vec<String> = if target == "aarch64-apple-ios-sim" {
        ios_sim_clang_args()
    } else {
        vec![]
    };

    // modules that need symbol renaming to avoid linker collisions with v1 counterparts.
    // Both cam_v1 and cam_v2 define identical C symbol names (e.g. asn_DEF_CAM,
    // asn_DEF_ItsPduHeader, ...); the linker would silently pick v1's definitions for all
    let rename_modules: &[&str] = &["cam_v2", "denm_v2"];

    let modules = vec![
        "cam_v1", "denm_v1",
        "cam_v2", "denm_v2", "cpm_v2", "is_v2"
    ];

    for module in &modules {
        let mod_dir = asn1_c_dir.join(module);
        if !mod_dir.exists() {
            continue;
        }

        let mut c_files = vec![];
        if let Ok(entries) = fs::read_dir(&mod_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().unwrap_or_default() == "c" {
                    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                    if file_name != "converter-example.c" {
                        c_files.push(path);
                    }
                }
            }
        }

        if c_files.is_empty() {
            continue;
        }

        // for conflicting v2 modules, generate a rename header so every asn_DEF_FOO becomes asn_DEF_FOO_v2
        let rename_header_path = if rename_modules.contains(module) {
            let suffix = module.replace("cam_", "").replace("denm_", ""); // "v2"
            let rename_path = out_dir.join(format!("{}_rename.h", module));
            let mut defines = String::new();

            for path in &c_files {
                if let Ok(content) = fs::read_to_string(path) {
                    for line in content.lines() {
                        if let Some(rest) = line.strip_prefix("asn_TYPE_descriptor_t asn_DEF_") {
                            let sym = rest
                                .split(|c: char| !c.is_alphanumeric() && c != '_')
                                .next()
                                .unwrap_or("")
                                .trim();
                            if !sym.is_empty() {
                                defines.push_str(&format!(
                                    "#define asn_DEF_{0} asn_DEF_{0}_{1}\n",
                                    sym, suffix
                                ));
                            }
                        }
                    }
                }
            }

            fs::write(&rename_path, defines).expect("failed to write rename header");
            Some(rename_path)
        } else {
            None
        };

        let mut build = cc::Build::new();
        build.files(c_files.clone())
            .include(&mod_dir)
            .flag_if_supported("-Wno-missing-field-initializers")
            .flag_if_supported("-Wno-uninitialized")
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-Wno-unused-variable")
            .flag_if_supported("-Wno-sign-compare")
            .flag_if_supported("-Wno-nonportable-include-path");

        if let Some(ref rpath) = rename_header_path {
            build.flag(&format!("-include{}", rpath.display()));
        }

        build.compile(&format!("etsi_v2x_{}", module));

        let wrapper_name = format!("etsi_{}_wrapper.h", module);
        let wrapper_path = mod_dir.join(&wrapper_name);

        let mut builder = bindgen::Builder::default()
            .header(wrapper_path.to_str().unwrap())
            .clang_arg(format!("-I{}", mod_dir.display()))
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

        for arg in &extra_clang_args {
            builder = builder.clang_arg(arg);
        }

        if let Some(ref rpath) = rename_header_path {
            builder = builder.clang_arg(format!("-include{}", rpath.display()));
        }

        let bindings = builder
            .generate()
            .expect(&format!("Unable to generate bindings for {}", module));

        bindings
            .write_to_file(out_dir.join(format!("{}_bindings.rs", module)))
            .expect(&format!("Couldn't write bindings for {}", module));
    }
}
