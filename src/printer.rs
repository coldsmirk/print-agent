use anyhow::Result;

pub fn list_printers() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::list_printers()
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::list_printers()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            "HP LaserJet Pro M404".into(),
            "Canon imageRUNNER 2530".into(),
            "Epson L360 Series".into(),
        ]
    }
}

pub fn list_paper_bins(printer: &str) -> Vec<String> {
    if printer.is_empty() {
        return default_paper_bins();
    }

    let _ = printer;
    default_paper_bins()
}

fn default_paper_bins() -> Vec<String> {
    vec![
        "自动".into(),
        "纸盒 1".into(),
        "纸盒 2".into(),
        "纸盒 3".into(),
        "手动进纸".into(),
    ]
}

pub fn print_document(
    printer: &str,
    data: &[u8],
    file_format: &str,
    duplex: bool,
    copies: u32,
) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::print_document(printer, data, file_format, duplex, copies)
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::print_document(printer, data, file_format, duplex, copies)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        tracing::info!(
            "Mock print: printer={printer}, format={file_format}, duplex={duplex}, copies={copies}, size={}bytes",
            data.len()
        );
        Ok(())
    }
}

fn format_extension(file_format: &str) -> &'static str {
    match file_format.to_uppercase().as_str() {
        "PDF" => "pdf",
        "DOCX" | "DOC" => "docx",
        "PNG" => "png",
        "JPG" | "JPEG" => "jpg",
        "BMP" => "bmp",
        _ => "tmp",
    }
}

fn write_temp_file(data: &[u8], extension: &str) -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir()
        .join(format!("print_agent_{}.{}", std::process::id(), extension));
    std::fs::write(&path, data)?;
    Ok(path)
}

fn schedule_temp_cleanup(path: std::path::PathBuf, delay_secs: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(delay_secs));
        std::fs::remove_file(path).ok();
    });
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use anyhow::{Result, bail};
    use windows::core::*;

    pub fn list_printers() -> Vec<String> {
        // Use PowerShell to enumerate printers (avoids unstable Win32 Printing API bindings)
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Get-Printer | Select-Object -ExpandProperty Name"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn print_document(
        _printer: &str,
        data: &[u8],
        file_format: &str,
        _duplex: bool,
        copies: u32,
    ) -> Result<()> {
        let ext = super::format_extension(file_format);
        let temp_file = super::write_temp_file(data, ext)?;

        for _ in 0..copies.max(1) {
            shell_print(&temp_file)?;
        }

        super::schedule_temp_cleanup(temp_file, 30);

        Ok(())
    }

    fn shell_print(file_path: &std::path::Path) -> Result<()> {
        use windows::Win32::UI::Shell::ShellExecuteW;

        let operation = HSTRING::from("print");
        let file = HSTRING::from(file_path.to_string_lossy().as_ref());

        unsafe {
            let result = ShellExecuteW(
                None,
                &operation,
                &file,
                None,
                None,
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );

            if result.0 as usize <= 32 {
                bail!("ShellExecuteW failed with code: {:?}", result.0);
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use anyhow::{Result, bail};
    use std::process::Command;

    pub fn list_printers() -> Vec<String> {
        let output = Command::new("lpstat").arg("-a").output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines()
                    .filter_map(|line| Some(line.split_whitespace().next()?.to_owned()))
                    .collect()
            }
            _ => {
                tracing::warn!("lpstat command failed, returning empty printer list");
                Vec::new()
            }
        }
    }

    pub fn print_document(
        printer: &str,
        data: &[u8],
        file_format: &str,
        duplex: bool,
        copies: u32,
    ) -> Result<()> {
        let ext = super::format_extension(file_format);
        let temp_file = super::write_temp_file(data, ext)?;

        let mut cmd = Command::new("lp");
        cmd.arg("-d").arg(printer);
        cmd.arg("-n").arg(copies.max(1).to_string());
        if duplex {
            cmd.arg("-o").arg("sides=two-sided-long-edge");
        }
        cmd.arg(&temp_file);

        let output = cmd.output()?;

        super::schedule_temp_cleanup(temp_file, 10);

        if output.status.success() {
            tracing::info!("macOS lp print submitted to {printer}");
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("lp command failed: {err}");
        }
    }
}
