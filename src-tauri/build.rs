fn main() {
    // tauri-build embeds its Common-Controls v6 app manifest into bin
    // targets only (embed-resource emits `rustc-link-arg-bins`). Test
    // binaries link the same dialog code (rfd's TaskDialogIndirect via
    // tauri-plugin-dialog), and without the manifest the loader binds
    // comctl32 5.82 — which lacks TaskDialogIndirect — so every test exe
    // dies with STATUS_ENTRY_POINT_NOT_FOUND before main.
    //
    // Fix: build the app without tauri's own manifest and embed the exact
    // same manifest ourselves for every link kind (bins, tests, …).
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(
            out_dir.join("app-manifest.xml"),
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#,
        )
        .unwrap();
        std::fs::write(out_dir.join("app-manifest.rc"), "1 24 \"app-manifest.xml\"\r\n").unwrap();
        embed_resource::compile_for_everything(out_dir.join("app-manifest.rc"), embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
    tauri_build::try_build(attrs).expect("failed to run tauri build");
}
