use zed_extension_api::{self as zed, LanguageServerId, Result, Worktree};

const GITHUB_REPOSITORY: &str = "loominatrx/zed-selene";

struct SeleneExtension {
    downloaded_adapter: Option<String>,
    downloaded_selene: Option<String>,
}

impl zed::Extension for SeleneExtension {
    fn new() -> Self {
        Self {
            downloaded_adapter: None,
            downloaded_selene: None,
        }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        // Prefer a local development binary; otherwise fetch the matching
        // prebuilt adapter from this extension's latest GitHub release.
        let command = if let Some(path) = worktree.which("zed-selene-lsp") {
            path
        } else if let Some(path) = &self.downloaded_adapter {
            path.clone()
        } else {
            let path = download_adapter()?;
            self.downloaded_adapter = Some(path.clone());
            path
        };
        let selene_path = if let Some(path) = worktree.which("selene") {
            path
        } else if let Some(path) = &self.downloaded_selene {
            path.clone()
        } else {
            let path = download_selene()?;
            self.downloaded_selene = Some(path.clone());
            path
        };
        let selene_path = if selene_path.starts_with('/') {
            selene_path
        } else {
            std::env::current_dir()
                .map_err(|error| format!("Could not locate downloaded Selene binary: {error}"))?
                .join(selene_path)
                .to_string_lossy()
                .into_owned()
        };
        Ok(zed::Command {
            command,
            args: vec![],
            env: vec![("SELENE_PATH".into(), selene_path)],
        })
    }
}

fn download_adapter() -> Result<String> {
    let (os, architecture) = zed::current_platform();
    let target = match (os, architecture) {
        (zed::Os::Linux, zed::Architecture::X8664) => "x86_64-unknown-linux-gnu",
        (zed::Os::Mac, zed::Architecture::Aarch64) => "aarch64-apple-darwin",
        (zed::Os::Mac, zed::Architecture::X8664) => "x86_64-apple-darwin",
        (zed::Os::Windows, zed::Architecture::X8664) => "x86_64-pc-windows-msvc",
        _ => {
            return Err(format!(
                "Unsupported Selene adapter platform: {os:?} {architecture:?}"
            ))
        }
    };
    let executable_suffix = if os == zed::Os::Windows { ".exe" } else { "" };
    let asset_name = format!("zed-selene-lsp-{target}{executable_suffix}");
    let release = zed::latest_github_release(
        GITHUB_REPOSITORY,
        zed::GithubReleaseOptions {
            require_assets: true,
            pre_release: false,
        },
    )?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("Release {} does not include {asset_name}", release.version))?;
    let path = format!("bin/{asset_name}");
    if std::fs::metadata(&path).is_ok() {
        return Ok(path);
    }
    std::fs::create_dir_all("bin")
        .map_err(|error| format!("Could not create adapter directory: {error}"))?;
    zed::download_file(
        &asset.download_url,
        &path,
        zed::DownloadedFileType::Uncompressed,
    )?;
    if os != zed::Os::Windows {
        zed::make_file_executable(&path)?;
    }
    Ok(path)
}

fn download_selene() -> Result<String> {
    let (os, architecture) = zed::current_platform();
    let platform = match (os, architecture) {
        (zed::Os::Linux, zed::Architecture::X8664) => "linux",
        (zed::Os::Mac, zed::Architecture::Aarch64) => "macos",
        (zed::Os::Windows, zed::Architecture::X8664) => "windows",
        _ => {
            return Err(format!(
                "Selene's upstream releases do not provide a binary for {os:?} {architecture:?}"
            ))
        }
    };
    let release = zed::latest_github_release(
        "Kampfkarren/selene",
        zed::GithubReleaseOptions {
            require_assets: true,
            pre_release: false,
        },
    )?;
    let asset_name = format!("selene-{}-{platform}.zip", release.version);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| {
            format!(
                "Selene release {} has no {asset_name} asset",
                release.version
            )
        })?;
    let version_directory = format!("selene-{}", release.version);
    let binary_name = if os == zed::Os::Windows {
        "selene.exe"
    } else {
        "selene"
    };
    let binary_path = format!("{version_directory}/{binary_name}");
    if std::fs::metadata(&binary_path).is_ok() {
        return Ok(binary_path);
    }
    zed::download_file(
        &asset.download_url,
        &version_directory,
        zed::DownloadedFileType::Zip,
    )?;
    if os != zed::Os::Windows {
        zed::make_file_executable(&binary_path)?;
    }
    Ok(binary_path)
}

zed::register_extension!(SeleneExtension);
