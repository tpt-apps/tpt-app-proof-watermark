//! TPT Proof Watermarker Pro — desktop shell (paid edition).
//!
//! A native host binary that opens the OS webview (WebView2 on Windows) and
//! serves the `trunk build --release` output of the appfront DOM bundle from
//! `dist/`. The same DOM bundle ships to the hub for the free WASM edition;
//! the Pro difference at runtime is that the bundle is built with
//! `--features pro`, which compiles in batch folder watermarking, saved
//! per-client presets, and explicit EXIF stripping.
//!
//! Beyond `open_external`, the shell implements the file-bridge IPC the
//! hosted page drives for batch runs (the webview sandbox has no filesystem
//! access of its own):
//!
//! - `dialog.open` — native folder/file picker (appfront builtin, rfd)
//! - `list_images` — flat listing of image files under a folder
//! - `read_file_b64` — image bytes in, base64 out
//! - `write_file_b64` — base64 in, file written under the active batch root
//!
//! Every read is restricted to image extensions, and every write must land
//! inside the most recently picked batch root (canonicalized + prefix
//! checked), so the hosted page can't be turned into a general file API.
//!
//! `dist/` is also embedded into the exe at compile time (see [`DIST`]), so
//! the file actually shipped to customers (the Gumroad zip) is the exe
//! alone. Resolve order for the bundle directory:
//! 1. `TPT_PROOF_DIST` environment variable (dev override),
//! 2. `./dist` next to the current working directory (dev),
//! 3. `dist` next to the executable (dev),
//! 4. the embedded copy, extracted once into the OS temp directory — the
//!    path a real end user's double-click takes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use include_dir::{include_dir, Dir};
use serde_json::json;
use tpt_appfront_webview::{
    open_dialog, Acl, AppBuilder, Capability, ParamKind, ParamSpec, WindowConfig,
};

const APP_ID: &str = "nz.co.tptsolutions.proof-watermarker-pro";

/// Extensions the bridge will read. Proof photos arrive as JPEG or PNG.
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png"];

static DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/../dist");

/// Shared state behind the file-bridge commands.
#[derive(Default)]
struct Bridge {
    /// Root of the most recent `list_images` pick; writes must stay inside it.
    batch_root: Mutex<Option<PathBuf>>,
}

fn main() {
    let dist_dir = resolve_dist_dir();
    if !dist_dir.join("index.html").exists() {
        eprintln!(
            "tpt-proof-watermarker-pro: no index.html under {} and embedded-asset extraction failed — \
             this build is missing its web bundle",
            dist_dir.display()
        );
        std::process::exit(1);
    }

    let bridge = std::sync::Arc::new(Bridge::default());

    let result = AppBuilder::new(APP_ID)
        .with_window(WindowConfig {
            id: "main".to_string(),
            title: "TPT Proof Watermarker Pro".to_string(),
            width: 1280,
            height: 900,
            dist_dir,
        })
        .with_single_instance(true)
        .with_builtin_capabilities()
        .with_acl(file_bridge_acl())
        .run_with_value_handler({
            let bridge = bridge.clone();
            move |action, params| handle_command(&bridge, action, params)
        });

    if let Err(error) = result {
        eprintln!("tpt-proof-watermarker-pro: {error:#}");
        std::process::exit(1);
    }
}

/// ACL for the file-bridge actions the page may call. `dialog.open` comes
/// from `with_builtin_capabilities`; everything here is our own surface.
fn file_bridge_acl() -> Acl {
    let cap = |action: &str, params: Vec<ParamSpec>| Capability {
        action: action.to_string(),
        params,
    };
    let path_param = || ParamSpec {
        name: "path".to_string(),
        required: true,
        kind: ParamKind::String,
        default: None,
    };
    Acl {
        capabilities: vec![
            cap("list_images", vec![path_param()]),
            cap("read_file_b64", vec![path_param()]),
            cap(
                "write_file_b64",
                vec![path_param(), ParamSpec {
                    name: "data".to_string(),
                    required: true,
                    kind: ParamKind::String,
                    default: None,
                }],
            ),
        ],
    }
}

fn handle_command(
    bridge: &Bridge,
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match action {
        "dialog.open" => Ok(open_dialog(&params)),
        "list_images" => {
            let path = param_path(&params)?;
            *bridge.batch_root.lock().unwrap() = Some(PathBuf::from(&path));
            let entries = list_images(Path::new(&path))
                .map_err(|e| format!("cannot list {path}: {e}"))?;
            Ok(json!({ "files": entries }))
        }
        "read_file_b64" => {
            let path = param_path(&params)?;
            let p = Path::new(&path);
            if !is_image_path(p) {
                return Err(format!("not a supported image type: {path}"));
            }
            let bytes = std::fs::read(p).map_err(|e| format!("cannot read {path}: {e}"))?;
            Ok(json!({
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            }))
        }
        "write_file_b64" => {
            let path = param_path(&params)?;
            let data = params
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "missing data".to_string())?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("bad base64: {e}"))?;
            write_under_batch_root(bridge, Path::new(&path), &bytes)?;
            Ok(json!({ "written": bytes.len() }))
        }
        "open_external" => {
            let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");
            // Belt-and-braces on top of the ACL's string-type check: only
            // ever hand the OS shell a tptsolutions.co.nz URL.
            if url.starts_with("https://tptsolutions.co.nz") {
                let _ = open::that(url);
            }
            Ok(serde_json::Value::Null)
        }
        other => Err(format!("unknown action {other}")),
    }
}

fn param_path(params: &serde_json::Value) -> Result<String, String> {
    params
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "missing path".to_string())
}

fn is_image_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Confine writes to the active batch root (canonicalized prefix check) so
/// the hosted page can't use the bridge as a general file-write API.
fn write_under_batch_root(bridge: &Bridge, dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let root = bridge
        .batch_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no batch folder selected".to_string())?;
    let root_canon = root.canonicalize().map_err(|e| e.to_string())?;
    let parent = dest
        .parent()
        .ok_or_else(|| "bad destination".to_string())?
        .to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| format!("cannot create folder: {e}"))?;
    let parent_canon = if parent.as_os_str().is_empty() {
        root_canon.clone()
    } else {
        parent
            .canonicalize()
            .map_err(|e| format!("bad destination folder: {e}"))?
    };
    if !parent_canon.starts_with(&root_canon) {
        return Err("destination is outside the selected folder".to_string());
    }
    std::fs::write(dest, bytes).map_err(|e| format!("cannot write {}: {e}", dest.display()))
}

fn list_image_paths(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() && is_image_path(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn list_images(dir: &Path) -> std::io::Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    for p in list_image_paths(dir) {
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        out.push(json!({
            "name": p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            "path": p.to_string_lossy(),
            "size": size,
        }));
    }
    Ok(out)
}

fn resolve_dist_dir() -> PathBuf {
    if let Ok(from_env) = std::env::var("TPT_PROOF_DIST") {
        return PathBuf::from(from_env);
    }
    let cwd_dist = PathBuf::from("dist");
    if cwd_dist.join("index.html").exists() {
        return cwd_dist;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let exe_dist = exe_dir.join("dist");
            if exe_dist.join("index.html").exists() {
                return exe_dist;
            }
        }
    }
    extract_embedded_dist()
}

/// Writes the embedded [`DIST`] contents to a stable temp directory (once —
/// subsequent launches reuse the extracted copy) and returns its path. This
/// is the path a customer's real double-click takes: no `dist/` folder
/// shipped or found, so the exe unpacks its own.
fn extract_embedded_dist() -> PathBuf {
    let out_dir = std::env::temp_dir().join(format!("{APP_ID}-dist"));
    if !out_dir.join("index.html").exists() {
        let _ = std::fs::create_dir_all(&out_dir);
        if let Err(error) = DIST.extract(&out_dir) {
            eprintln!("tpt-proof-watermarker-pro: failed to extract embedded assets: {error:#}");
        }
    }
    out_dir
}
