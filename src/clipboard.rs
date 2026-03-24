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

pub fn resolve_link_path(link: &str) -> Option<String> {
    let vault = std::env::var("WDTTG_VAULT").ok()?;

    let link = link.trim_start_matches("[[").trim_end_matches("]]");
    let file = link.split('#').next()?;

    let file = if file.ends_with(".md") {
        file.to_string()
    } else {
        format!("{}.md", file)
    };

    Some(format!("{}/{}", vault, file))
}

pub enum OpenLinkResult {
    OpenedUrl,
    CopiedPath(String),
    CopiedRaw(String),
}

pub fn open_link(link: &str) -> Result<OpenLinkResult, String> {
    let link = link.trim();

    if link.starts_with("http://") || link.starts_with("https://") {
        open_url(link)?;
        Ok(OpenLinkResult::OpenedUrl)
    } else if link.starts_with("[[") && link.ends_with("]]") {
        let path = resolve_link_path(link).ok_or("WDTTG_VAULT not set")?;
        copy_to_clipboard(&path)?;
        Ok(OpenLinkResult::CopiedPath(path))
    } else {
        copy_to_clipboard(link)?;
        Ok(OpenLinkResult::CopiedRaw(link.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static VAULT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_resolve_link_path_simple() {
        let _lock = VAULT_LOCK.lock().unwrap();
        std::env::set_var("WDTTG_VAULT", "/test/vault");
        let result = resolve_link_path("[[daily/2026-03-24]]");
        assert_eq!(result, Some("/test/vault/daily/2026-03-24.md".to_string()));
        std::env::remove_var("WDTTG_VAULT");
    }

    #[test]
    fn test_resolve_link_path_with_section() {
        let _lock = VAULT_LOCK.lock().unwrap();
        std::env::set_var("WDTTG_VAULT", "/test/vault");
        let result = resolve_link_path("[[projects/feature-x#Implementation]]");
        assert_eq!(
            result,
            Some("/test/vault/projects/feature-x.md".to_string())
        );
        std::env::remove_var("WDTTG_VAULT");
    }

    #[test]
    fn test_resolve_link_path_with_md_extension() {
        let _lock = VAULT_LOCK.lock().unwrap();
        std::env::set_var("WDTTG_VAULT", "/test/vault");
        let result = resolve_link_path("[[notes/meeting.md]]");
        assert_eq!(result, Some("/test/vault/notes/meeting.md".to_string()));
        std::env::remove_var("WDTTG_VAULT");
    }

    #[test]
    fn test_resolve_link_path_no_vault() {
        let _lock = VAULT_LOCK.lock().unwrap();
        std::env::remove_var("WDTTG_VAULT");
        let result = resolve_link_path("[[daily/2026-03-24]]");
        assert_eq!(result, None);
    }
}
