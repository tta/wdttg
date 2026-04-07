use std::io::Write;
use std::process::Command;

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let result = if cfg!(target_os = "macos") {
        Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            })
    } else if cfg!(target_os = "linux") {
        Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            })
    } else {
        return Err("Unsupported platform for clipboard".to_string());
    };

    result.map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
    Ok(())
}

pub enum OpenLinkResult {
    OpenedUrl,
    OpenInEditor { link: String },
    CopiedPath(String),
    CopiedRaw(String),
}

pub fn open_link(link: &str) -> Result<OpenLinkResult, String> {
    let link = link.trim();

    if link.starts_with("http://") || link.starts_with("https://") {
        open_url(link)?;
        Ok(OpenLinkResult::OpenedUrl)
    } else if link.starts_with("[[") && link.ends_with("]]") {
        Ok(OpenLinkResult::OpenInEditor {
            link: link.to_string(),
        })
    } else {
        copy_to_clipboard(link)?;
        Ok(OpenLinkResult::CopiedRaw(link.to_string()))
    }
}

pub fn open_in_nvim(link: &str) -> Result<(), String> {
    let mut cmd = Command::new("nvim");
    cmd.args([
        "-c",
        "lua require('lazy').load({plugins='obsidian.nvim'})",
        "-c",
        &format!("put ='{}'", link),
        "-c",
        "ObsidianFollowLink",
        "-c",
        "bdelete! 1",
    ]);

    let result = cmd
        .status()
        .map_err(|e| format!("Failed to open editor: {}", e))?;

    if result.success() {
        Ok(())
    } else {
        Err(format!("Editor exited with: {}", result))
    }
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        return Err("Unsupported platform".to_string());
    }
    Ok(())
}

