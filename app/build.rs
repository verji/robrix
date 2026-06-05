fn main() {
    // Embed the Windows application manifest into the executable.
    // This declares Windows 10 compatibility, which CEF's GPU process
    // requires for proper Direct3D initialization.
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile("robrix.rc", embed_resource::NONE);
    }
}
