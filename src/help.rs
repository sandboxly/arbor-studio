//! Opens the online help (the `docs` folder, published to GitHub Pages) in the default web browser.

/// Where the help is published. `ARBOR_HELP_URL` points the app at another copy,
/// for example a local folder (`file:///…/docs/`) while writing the pages.
pub const HELP_URL: &str = "https://sandboxly.github.io/arbor-studio/";

/// The address of one help page, for example `shortcuts.html` or `lod.html#billboard`.
pub fn url(page: &str) -> String {
    let base = std::env::var("ARBOR_HELP_URL").unwrap_or_else(|_| HELP_URL.to_string());
    let base = if base.ends_with('/') { base } else { format!("{base}/") };
    let page = page.trim_start_matches('/');
    if page == "index.html" { base } else { format!("{base}{page}") }
}

/// Opens one help page in the default browser and returns its address.
pub fn open(page: &str) -> Result<String, String> {
    let address = url(page);
    launch(&address).map_err(|e| format!("Could not open the web browser: {e}"))?;
    Ok(address)
}

fn launch(address: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(address);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // rundll32 hands the address to the default browser without a console window or
        // the quoting rules of `cmd /C start`.
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", address]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(address);
        c
    };
    // The opener returns quickly; it is not waited for, so the window never blocks.
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_addresses() {
        assert_eq!(url("index.html"), HELP_URL);
        assert_eq!(url("lod.html#billboard"), format!("{HELP_URL}lod.html#billboard"));
    }
}
