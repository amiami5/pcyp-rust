//! ビルドのときに、アイコンとファイル情報 (説明・版) を exe に埋め込む。
//!
//! 埋め込まないと、エクスプローラーやタスクバーのピン留めで標準のアイコン (白い紙) になる。
//! アイコンは src/icon.rs の描き方で作るので、窓のアイコンと必ず同じになる。

#[path = "icon.rs"]
#[allow(dead_code)]
mod icon;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=src/build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("app.ico");
    std::fs::write(&ico, icon::ico_file(&[16, 20, 24, 32, 40, 48, 64, 256])).unwrap();

    let v = |name: &str| std::env::var(name).unwrap();
    let (major, minor, patch) = (v("CARGO_PKG_VERSION_MAJOR"), v("CARGO_PKG_VERSION_MINOR"), v("CARGO_PKG_VERSION_PATCH"));
    let version = v("CARGO_PKG_VERSION");
    let name = v("CARGO_PKG_NAME");
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{ico}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041104B0"
    BEGIN
      VALUE "FileDescription", "{name}"
      VALUE "ProductName", "{name}"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "{name}.exe"
      VALUE "InternalName", "{name}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0411, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "\\\\"),
    );
    let rc_path = out.join("app.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_optional().unwrap();
}
