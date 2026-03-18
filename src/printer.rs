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

    pub fn print_document(
        printer: &str,
        data: &[u8],
        file_format: &str,
        _duplex: bool,
        copies: u32,
    ) -> Result<()> {
        let fmt = file_format.to_uppercase();
        match fmt.as_str() {
            "PDF" | "PNG" | "JPG" | "JPEG" | "BMP" | "GIF" => {
                // RAW spooler print: silent, no application window
                for _ in 0..copies.max(1) {
                    spooler_print(printer, data, &fmt)?;
                }
                Ok(())
            }
            _ => {
                // Fallback: ShellExecuteW for formats that need an application to render
                let ext = super::format_extension(file_format);
                let temp_file = super::write_temp_file(data, ext)?;
                for _ in 0..copies.max(1) {
                    shell_print(&temp_file)?;
                }
                super::schedule_temp_cleanup(temp_file, 30);
                Ok(())
            }
        }
    }

    /// Send raw data to printer via Print Spooler API (silent, no UI)
    fn spooler_print(printer: &str, data: &[u8], data_type: &str) -> Result<()> {
        let printer_name = HSTRING::from(printer);
        let mut handle = PRINTER_HANDLE::default();

        unsafe {
            OpenPrinterW(&printer_name, &mut handle, None)
                .context("OpenPrinterW failed")?;
        }

        // Map format to spooler data type
        let spool_type = match data_type {
            "PDF" => "RAW",
            _ => "RAW", // Images also sent as RAW; printer handles rendering
        };

        let doc_name = HSTRING::from("PrintAgent Job");
        let data_type_w = HSTRING::from(spool_type);
        let doc_info = DOC_INFO_1W {
            pDocName: PWSTR(doc_name.as_ptr() as *mut _),
            pOutputFile: PWSTR::null(),
            pDatatype: PWSTR(data_type_w.as_ptr() as *mut _),
        };

        let job_id = unsafe { StartDocPrinterW(handle, 1, &doc_info as *const _ as *const _) };
        if job_id == 0 {
            unsafe { let _ = ClosePrinter(handle); }
            bail!("StartDocPrinterW failed");
        }

        let page_ok = unsafe { StartPagePrinter(handle) };
        if !page_ok.as_bool() {
            unsafe {
                EndDocPrinter(handle);
                let _ = ClosePrinter(handle);
            }
            bail!("StartPagePrinter failed");
        }

        let mut written: u32 = 0;
        let write_ok = unsafe {
            WritePrinter(
                handle,
                data.as_ptr() as *const _,
                data.len() as u32,
                &mut written,
            )
        };

        unsafe {
            EndPagePrinter(handle);
            EndDocPrinter(handle);
            let _ = ClosePrinter(handle);
        }

        if !write_ok.as_bool() {
            bail!("WritePrinter failed");
        }

        Ok(())
    }

    fn shell_print(file_path: &std::path::Path) -> Result<()> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

        let operation = HSTRING::from("print");
        let file = HSTRING::from(file_path.to_string_lossy().as_ref());

        let result = unsafe {
            ShellExecuteW(None, &operation, &file, None, None, SW_HIDE)
        };

        if result.0 as usize <= 32 {
            bail!("ShellExecuteW failed with code: {:?}", result.0);
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
