fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Debug builds keep Slint debug info so UI tests can locate and measure elements.
    let config = slint_build::CompilerConfiguration::new()
        .with_debug_info(std::env::var("PROFILE").as_deref() == Ok("debug"));
    slint_build::compile_with_config("ui/main.slint", config)?;
    Ok(())
}
