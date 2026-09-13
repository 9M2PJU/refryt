//! Embeds `refryt.ico` as the .exe's own icon resource — Windows only, and specifically the
//! *target* platform, not the host running this build script: cross-compiling for Windows from
//! Linux still needs this to run, so the check is `CARGO_CFG_TARGET_OS` rather than
//! `cfg(windows)`, which would reflect the host instead and never fire at all.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("refryt.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
