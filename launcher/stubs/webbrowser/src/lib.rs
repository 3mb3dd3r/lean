//! Stub; see Cargo.toml.
pub fn open(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "opening links is not supported"))
}
