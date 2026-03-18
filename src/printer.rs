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

    // TODO: query real paper bins via Win32 DeviceCapabilitiesW when API stabilizes
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

/// Print from file bytes. Writes to temp file then prints.
pub fn print_bytes(
    printer: &str,
    data: &[u8],
    file_format: &str,
    duplex: bool,
    copies: u32,
) -> Result<()> {
    let ext = format_extension(file_format);
    let temp = write_temp_file(data, ext)?;
    let result = print_file(printer, &temp, duplex, copies);
    schedule_temp_cleanup(temp, 60);
    result
}

/// Print an existing file on disk.
pub fn print_file(
    printer: &str,
    path: &std::path::Path,
    duplex: bool,
    copies: u32,
) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::print_file(printer, path, duplex, copies)
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::print_file(printer, path, duplex, copies)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        tracing::info!(
            "Mock print: printer={printer}, file={}, duplex={duplex}, copies={copies}",
            path.display()
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
    use anyhow::{Result, Context, bail};
    use windows::core::*;
    use windows::Win32::Graphics::Printing::*;

    pub fn list_printers() -> Vec<String> {
        enumerate_printers().unwrap_or_default()
    }

    fn enumerate_printers() -> Result<Vec<String>> {
        let mut needed: u32 = 0;
        let mut returned: u32 = 0;

        // First call to get required buffer size
        let _ = unsafe {
            EnumPrintersW(
                PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
                None,
                2,
                None,
                &mut needed,
                &mut returned,
            )
        };

        if needed == 0 {
            return Ok(Vec::new());
        }

        let mut buffer = vec![0u8; needed as usize];

        unsafe {
            EnumPrintersW(
                PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS,
                None,
                2,
                Some(&mut buffer),
                &mut needed,
                &mut returned,
            )
            .context("EnumPrintersW failed")?;
        }

        let infos = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr() as *const PRINTER_INFO_2W,
                returned as usize,
            )
        };

        Ok(infos
            .iter()
            .filter(|info| !info.pPrinterName.is_null())
            .filter_map(|info| unsafe { info.pPrinterName.to_string().ok() })
            .collect())
    }

    pub fn print_file(
        printer: &str,
        path: &std::path::Path,
        duplex: bool,
        copies: u32,
    ) -> Result<()> {
        if let Err(e) = set_printer_devmode(printer, duplex, copies) {
            tracing::warn!("Failed to set printer DEVMODE: {e}");
        }
        shell_print(path, printer)
    }

    fn set_printer_devmode(printer: &str, duplex: bool, copies: u32) -> Result<()> {
        use windows::Win32::Graphics::Gdi::*;

        let printer_name = HSTRING::from(printer);
        let mut handle = PRINTER_HANDLE::default();
        unsafe {
            OpenPrinterW(&printer_name, &mut handle, None)
                .context("OpenPrinterW failed")?;
        }

        // Query required buffer size (fmode = 0)
        let size = unsafe {
            DocumentPropertiesW(None, handle, &printer_name, None, None, 0)
        };
        if size <= 0 {
            unsafe { let _ = ClosePrinter(handle); }
            bail!("DocumentPropertiesW size query failed");
        }

        let mut buf = vec![0u8; size as usize];
        let dm = buf.as_mut_ptr() as *mut DEVMODEW;

        // Get current DEVMODE
        let ret = unsafe {
            DocumentPropertiesW(None, handle, &printer_name, Some(dm), None, DM_OUT_BUFFER.0)
        };
        if ret < 0 {
            unsafe { let _ = ClosePrinter(handle); }
            bail!("DocumentPropertiesW get failed");
        }

        // Modify duplex and copies
        unsafe {
            (*dm).dmFields |= DM_DUPLEX | DM_COPIES;
            (*dm).dmDuplex = if duplex { DMDUP_VERTICAL } else { DMDUP_SIMPLEX };
            (*dm).Anonymous1.Anonymous1.dmCopies = copies.min(999) as i16;
        }

        // Apply modified DEVMODE
        let ret = unsafe {
            DocumentPropertiesW(
                None, handle, &printer_name,
                Some(dm), Some(dm as *const _),
                (DM_IN_BUFFER | DM_OUT_BUFFER).0,
            )
        };
        unsafe { let _ = ClosePrinter(handle); }

        if ret < 0 {
            bail!("DocumentPropertiesW apply failed");
        }
        Ok(())
    }

    /// Use "printto" verb to print to a specific printer with SW_HIDE
    fn printto(file_path: &std::path::Path, printer: &str) -> Result<()> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

        let file = HSTRING::from(file_path.to_string_lossy().as_ref());
        let printer_param = HSTRING::from(format!("\"{printer}\""));

        // Try "printto" first (specifies printer, quieter)
        let result = unsafe {
            ShellExecuteW(None, &HSTRING::from("printto"), &file, &printer_param, None, SW_HIDE)
        };
        if result.0 as usize > 32 {
            return Ok(());
        }

        // Fallback to "print" if "printto" is not registered for this file type
        tracing::info!("printto not supported, falling back to print verb");
        let result = unsafe {
            ShellExecuteW(None, &HSTRING::from("print"), &file, None, None, SW_HIDE)
        };
        if result.0 as usize > 32 {
            return Ok(());
        }

        bail!("ShellExecuteW failed with code: {:?}", result.0);
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

    pub fn print_file(
        printer: &str,
        path: &std::path::Path,
        duplex: bool,
        copies: u32,
    ) -> Result<()> {
        let mut cmd = Command::new("lp");
        cmd.arg("-d").arg(printer);
        cmd.arg("-n").arg(copies.max(1).to_string());
        if duplex {
            cmd.arg("-o").arg("sides=two-sided-long-edge");
        }
        cmd.arg(path);

        let output = cmd.output()?;

        if output.status.success() {
            tracing::info!("macOS lp print submitted to {printer}");
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("lp command failed: {err}");
        }
    }
}
