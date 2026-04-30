"""
Plugin: Install Mods & Scan FOMOD Choices
Authors: MaskPlague & Griffin

This plugin enables the sequential installation of multiple mod archives
for a mod organizer using a C++ DLL for installation.
It sets up logging, registers a C++ log callback, and handles mod installations
by scanning for the required DLL in common directories.

Additionally provides a "Scan FOMOD Choices" tool that infers FOMOD installation
selections for all installed mods by comparing files against FOMOD XML structure.
"""

import mobase
import platform
import logging
import time
import threading
import os
import re
import shutil
import sys
import ctypes
import json
import configparser
import datetime

from pathlib import Path
from mobase import GuessedString


class InstallError(RuntimeError):
    """Raised by :meth:`InstallMods.install` when the C++ side reports failure.

    Carries the error message returned by the DLL so callers can log or display
    it. Distinguishes a hard install failure (where ``installSucceeded()``
    returned False) from generic exceptions raised by the surrounding code.
    """

from PyQt6.QtCore import QCoreApplication, Qt, qDebug
from PyQt6.QtGui import QIcon
from PyQt6.QtWidgets import QFileDialog, QMessageBox, QProgressDialog

# ------------------------------------------------------------------------------
# Logger Initialization
# ------------------------------------------------------------------------------
# Create a logger for this module and set the logging level to DEBUG.
logger = logging.getLogger(__name__)
logger.setLevel(logging.DEBUG)

# Clear any existing logging handlers to ensure a clean configuration.
if logger.hasHandlers():
    logger.handlers.clear()

# Configure a default file handler.
# The log file lives next to this plugin file (under <MO2 plugins>/logs/),
# regardless of the working directory MO2 was launched from.
default_log_file = Path(__file__).parent / "logs" / "mo_salma.log"
default_log_file.parent.mkdir(parents=True, exist_ok=True)
default_handler = logging.FileHandler(str(default_log_file))
formatter = logging.Formatter('%(asctime)s - %(levelname)s - %(message)s')
default_handler.setFormatter(formatter)
logger.addHandler(default_handler)

# Log a test message to confirm that logging is working correctly.
logger.debug("Logger configured successfully.")

# ------------------------------------------------------------------------------
# Callback Function Definition
# ------------------------------------------------------------------------------
# Define the callback function type that matches the expected C++ signature.
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

# Major version of the DLL ABI this plugin is written against. Bumped only
# on incompatible C-API changes (added/removed exports, signature changes).
# A mismatch refuses to use the DLL rather than risk silent ABI drift.
EXPECTED_API_MAJOR = "1"

# ------------------------------------------------------------------------------
# Shared DLL Utilities
# ------------------------------------------------------------------------------
def _safe_path_exists(p: str) -> bool:
    """Check if a path exists, returning False on permission errors."""
    try:
        return Path(p).exists()
    except OSError:
        return False


def find_dll(dll_name="mo2-salma.dll"):
    """Locate the required DLL. Shared by all plugin classes."""
    search_paths = [
        Path(__file__).parent / "salma",
        Path.cwd(),
        Path(Path.cwd() / r"dlls\salma"),
        Path(__file__).parent,
        *[Path(p) for p in os.getenv("PATH", "").split(os.pathsep) if _safe_path_exists(p)]
    ]
    for path in search_paths:
        dll_path = path / dll_name
        try:
            if dll_path.exists():
                return dll_path.resolve()
        except OSError:
            continue
    raise FileNotFoundError(f"Could not find {dll_name} in common locations.")


_dll_cache = {}


def _configure_dll(lib):
    """Set ctypes signatures for every DLL export this plugin calls.

    Owned-string returns (install / installWithConfig / inferFomodSelections)
    are declared as ``c_void_p`` so we get the raw heap pointer back from
    ctypes and can hand it to freeResult after copying the bytes. Declaring
    them as ``c_char_p`` (the previous behaviour) lets ctypes auto-decode but
    silently leaks the underlying ``_strdup`` allocation per call.
    """
    lib.getApiVersion.argtypes = []
    lib.getApiVersion.restype = ctypes.c_char_p  # static const char*, never freed

    lib.freeResult.argtypes = [ctypes.c_void_p]
    lib.freeResult.restype = None

    lib.installSucceeded.argtypes = []
    lib.installSucceeded.restype = ctypes.c_bool

    lib.setLogCallback.argtypes = [CALLBACK_TYPE]
    lib.setLogCallback.restype = None

    lib.install.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.install.restype = ctypes.c_void_p

    lib.installWithConfig.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.installWithConfig.restype = ctypes.c_void_p

    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_void_p

    # resolveModArchive was added in salma DLL 1.1.0. Older deployed DLLs
    # do not export it; we leave the attribute unset and fall back to the
    # legacy Python resolution path in resolve_mod_archive() below.
    if hasattr(lib, "resolveModArchive"):
        lib.resolveModArchive.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
        lib.resolveModArchive.restype = ctypes.c_void_p


def _check_api_version(lib):
    """Verify the DLL's ABI matches what this plugin was written for.

    Pre-versioning DLLs (before getApiVersion was exported) raise
    AttributeError on the lookup; treat that as an incompatible deploy
    rather than continuing into UB.
    """
    try:
        version_bytes = lib.getApiVersion()
    except AttributeError as exc:
        msg = ("salma DLL is missing getApiVersion(); refusing to use to avoid ABI drift. "
               "Update to a newer DLL build that ships alongside this plugin.")
        logger.error(msg)
        raise RuntimeError(msg) from exc
    if not version_bytes:
        msg = "salma DLL returned empty getApiVersion(); refusing to use."
        logger.error(msg)
        raise RuntimeError(msg)
    version_str = version_bytes.decode("utf-8")
    major = version_str.split(".", 1)[0]
    if major != EXPECTED_API_MAJOR:
        msg = (f"salma DLL ABI mismatch: plugin expects major {EXPECTED_API_MAJOR}, "
               f"DLL reports {version_str}. Update the plugin or DLL.")
        logger.error(msg)
        raise RuntimeError(msg)
    logger.info(f"salma DLL ABI version: {version_str}")


def load_dll(dll_name="mo2-salma.dll"):
    """Load, configure, and cache the DLL. Returns the ctypes library handle.

    First call configures argtypes/restypes for every export and verifies the
    ABI version; subsequent calls hand back the cached handle.
    """
    if dll_name not in _dll_cache:
        dll_path = find_dll(dll_name)
        logger.info(f"Loading DLL: {dll_path}")
        lib = ctypes.CDLL(str(dll_path))
        _configure_dll(lib)
        _check_api_version(lib)
        _dll_cache[dll_name] = lib
    return _dll_cache[dll_name]


def _call_owned_string(lib, fn, *args) -> str:
    """Invoke a DLL function returning an owned C string (`_strdup` result).

    Decodes UTF-8 and always calls ``freeResult`` on the underlying pointer
    -- including if decoding throws -- so the heap allocation is not leaked.
    Returns "" if the function returned a null pointer.
    """
    addr = fn(*args)
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


def infer_fomod_selections(archive_path: str, mod_path: str) -> str:
    """Call the DLL's inferFomodSelections function and return the JSON string."""
    lib = load_dll()
    return _call_owned_string(
        lib,
        lib.inferFomodSelections,
        archive_path.encode("utf-8"),
        mod_path.encode("utf-8"))


def resolve_mod_archive(installation_file: str, mod_folder: str, mods_dir: str) -> str:
    """Resolve a mod's archive path using the same fallback chain as the dashboard.

    Delegates to the DLL's ``resolveModArchive`` (added in salma 1.1.0) when
    available. Older deployed DLLs lack the export, in which case we fall
    back to a downloads-dir-only Python resolution that matches the prior
    plugin behaviour - just enough to keep older deploys working until they
    update.
    """
    if not installation_file:
        return ""

    archive = Path(installation_file)
    if archive.is_absolute() and archive.exists():
        return str(archive)

    lib = load_dll()
    if hasattr(lib, "resolveModArchive"):
        return _call_owned_string(
            lib,
            lib.resolveModArchive,
            installation_file.encode("utf-8"),
            mod_folder.encode("utf-8"),
            mods_dir.encode("utf-8"))

    # Legacy fallback for pre-1.1.0 DLLs: only the downloads directory is
    # searched. This is intentionally conservative and lossy compared with
    # the C-API path which also tries mods-dir siblings.
    downloads_dir = os.environ.get("SALMA_DOWNLOADS_PATH", "")
    if downloads_dir:
        candidate = Path(downloads_dir) / archive
        if candidate.exists():
            return str(candidate)
    return ""


def get_archive_for_mod(mod_path: str) -> str:
    """Read meta.ini to find the installationFile for this mod."""
    meta_ini = Path(mod_path) / "meta.ini"
    if not meta_ini.exists():
        return ""

    config = configparser.ConfigParser()
    try:
        config.read(str(meta_ini), encoding="utf-8")
    except Exception:
        return ""

    # The archive path is stored under [General] installationFile
    archive = config.get("General", "installationFile", fallback="")
    return archive


_FOMOD_OUTPUT_MOD = "Salma FOMODs Output"


def _get_fomod_output_dir(organizer) -> Path:
    """Return (and create) the centralized FOMOD output folder."""
    mod_dir = Path(organizer.modsPath()) / _FOMOD_OUTPUT_MOD
    output_dir = mod_dir / "fomods"
    output_dir.mkdir(parents=True, exist_ok=True)
    meta_ini = mod_dir / "meta.ini"
    if not meta_ini.exists():
        meta_ini.write_text(
            "[General]\nmodid=0\nversion=\nnewestVersion=\n"
            "category=0\ninstallationFile=\nrepository=\n",
            encoding="utf-8")
    return output_dir


def _read_json_metadata(path: Path) -> dict:
    """Return the 'metadata' block from a choices JSON, or an empty dict.

    Handles legacy JSONs (no metadata block), missing/corrupt files, and
    ill-typed values without raising.
    """
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(data, dict):
        return {}
    meta = data.get("metadata", {})
    return meta if isinstance(meta, dict) else {}


def _inject_choice_metadata(parsed: dict, archive_path: str, module_name: str) -> None:
    """Mirror of Mo2FomodController.cpp's inject_choice_metadata.

    Adds a `metadata` block to the parsed FOMOD-choices JSON so future
    lookups can match by (modid, fileid) or (size, mtime) instead of fuzzy
    filename comparison. Schema must stay aligned with the C++ writer.
    """
    archive = Path(archive_path)
    nexus = _parse_nexus_filename(archive.stem)
    metadata = {
        "module_name": module_name,
        "archive_path": str(archive),
        "modid": nexus.get("modid", ""),
        "fileid": nexus.get("fileid", ""),
        "scanned_at": datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"),
    }
    try:
        stat = archive.stat()
        metadata["archive_size"] = stat.st_size
        metadata["archive_mtime"] = int(stat.st_mtime)
    except OSError:
        metadata["archive_size"] = 0
        metadata["archive_mtime"] = 0
    parsed["metadata"] = metadata


def _find_fomod_json(organizer, mod_name: str, archive_path: str = "") -> str:
    """Find a FOMOD choices JSON for the given mod name / archive.

    Lookup priority (first hit wins):
      1. (modid, fileid) parsed from the archive filename, matched against
         the metadata block in each JSON. Definitive for Nexus archives.
      2. (archive_size, archive_mtime) of the archive on disk, matched
         against the metadata block. Cheap fingerprint for non-Nexus mods.
      3. Exact filename match: `<mod_name>.json`. Backward compatible with
         JSONs scanned before the metadata block existed.
      4. Deprecated stem-prefix fuzzy match. Logs a warning so users know
         to re-scan to populate metadata.
    Returns the path as a string, or empty string if nothing matches.
    """
    output_dir = _get_fomod_output_dir(organizer)
    candidates = sorted(output_dir.glob("*.json"),
                        key=lambda p: len(p.stem), reverse=True)

    # 1. Match by (modid, fileid) parsed from the current archive filename.
    if archive_path:
        nexus = _parse_nexus_filename(Path(archive_path).stem)
        target_modid = nexus.get("modid", "")
        target_fileid = nexus.get("fileid", "")
        if target_modid and target_fileid:
            for candidate in candidates:
                meta = _read_json_metadata(candidate)
                if (meta.get("modid") == target_modid
                        and meta.get("fileid") == target_fileid):
                    return str(candidate)

    # 2. Match by (size, mtime) of the archive on disk.
    if archive_path:
        try:
            stat = Path(archive_path).stat()
        except OSError:
            stat = None
        if stat is not None:
            target_size = stat.st_size
            target_mtime = int(stat.st_mtime)
            for candidate in candidates:
                meta = _read_json_metadata(candidate)
                if (meta.get("archive_size") == target_size
                        and abs(int(meta.get("archive_mtime", 0)) - target_mtime) <= 1):
                    return str(candidate)

    # 3. Exact filename match (legacy / cosmetic naming convention).
    exact = output_dir / f"{mod_name}.json"
    if exact.exists():
        return str(exact)

    # 4. Deprecated: longest-stem-first prefix match.
    lower_name = mod_name.lower()
    for candidate in candidates:
        if lower_name.startswith(candidate.stem.lower()):
            logger.warning(
                f"[install] _find_fomod_json fell through to fuzzy stem match: "
                f"'{mod_name}' -> '{candidate.name}'. "
                f"Re-scan to populate metadata so future lookups are deterministic.")
            return str(candidate)
    return ""


def _parse_nexus_filename(filename: str) -> dict:
    """Parse Nexus-style filename: Name-ModID-Version-FileID."""
    match = re.match(r'^(.+?)-(\d+)-(.*)-(\d+)$', filename)
    if match:
        return {
            'modid': match.group(2),
            'version': match.group(3).replace('-', '.'),
            'fileid': match.group(4),
        }
    return {}


def _write_mod_meta_ini(mod_dir: Path, archive_path: str, mod_name: str):
    """Write meta.ini for an installed mod with Nexus metadata."""
    meta = _parse_nexus_filename(mod_name)
    modid = meta.get('modid', '0')
    fileid = meta.get('fileid', '0')
    version = meta.get('version', '')
    install_file = archive_path.replace('\\', '/')

    content = (
        "[General]\n"
        f"modid={modid}\n"
        f"version={version}\n"
        "newestVersion=\n"
        "category=0\n"
        f"installationFile={install_file}\n"
        "repository=\n"
        "\n"
        "[installedFiles]\n"
        "size=0\n"
    )
    if modid != '0':
        content += f"1\\modid={modid}\n"
        content += f"1\\fileid={fileid}\n"

    (mod_dir / "meta.ini").write_text(content, encoding="utf-8")

# ------------------------------------------------------------------------------
# Plugin Class: InstallMods
# ------------------------------------------------------------------------------
class InstallMods(mobase.IPluginTool):
    """
    Plugin tool for installing mod archives.

    This class implements the IPluginTool interface required by the mod organizer.
    It integrates with a C++ DLL to perform installations and manages the
    installation queue along with logging for each mod installation.
    """

    def __init__(self):
        super(InstallMods, self).__init__()
        self._parentWidget = None  # Reference to the parent GUI widget (if needed)
        self._log_callback = None  # Store the log callback instance to prevent garbage collection

    def init(self, organiser=mobase.IOrganizer, manager=mobase.IInstallationManager):
        """
        Initialize the plugin with references to the mod organizer and installation manager.

        Configures internal variables and registers the logging callback with the DLL.
        Logs system information for debugging purposes.
        """
        self.debug = True
        self.num = 0
        self._modList = []
        self.finished = True
        self._organizer = organiser
        self._manager = manager
        self._queue = []
        self.handler = None

        # Retrieve the last used download path from plugin settings.
        self.downloadLocation = self._organizer.pluginSetting(self.name(), "LastPath")

        # Log system and Python environment details.
        logger.info(sys.version)
        logger.info(sys.executable)
        logger.info(os.getcwd())
        logger.info(platform.architecture())

        # Register the Python log callback with the C++ DLL.
        self.setLog()
        return True

    @staticmethod
    def log_callback(message: bytes):
        """
        Callback function to log messages from the C++ DLL.

        Converts the received C-style string (bytes) into a Python string and logs it.
        Also forwards to MO2's application log window via qDebug.
        """
        text = message.decode("utf-8")
        logger.info(text)
        qDebug(text.encode("ascii", "replace").decode("ascii"))

    def setLog(self):
        """
        Register the Python log callback with the C++ DLL.

        This method loads the DLL (which configures argtypes/restypes once on
        first load) and registers the callback function. The callback instance
        is retained on ``self`` to keep ctypes from garbage-collecting it.
        """
        try:
            c_callback = CALLBACK_TYPE(InstallMods.log_callback)
            lib = load_dll()
            lib.setLogCallback(c_callback)
            self._log_callback = c_callback
            logger.debug("Log callback registered successfully.")
        except Exception as e:
            logger.error("Failed to set log callback: " + str(e))

    def install(self, archive_path: str, install_path: str, json_path: str = "", dll_name="mo2-salma.dll") -> str:
        """
        Call the DLL's 'installWithConfig' function to perform a mod installation.

        If json_path points to an existing FOMOD choices file, the DLL will apply
        those selections via ModuleConfig.xml. Otherwise it falls back to default behaviour.

        Args:
            archive_path (str): The path to the mod archive.
            install_path (str): The target installation directory.
            json_path (str): Optional path to a FOMOD choices JSON file.
            dll_name (str): Name of the DLL file to use.

        Returns:
            str: The install path returned by the C++ side on success.

        Raises:
            FileNotFoundError: If either the archive or installation path does not exist.
            InstallError: If ``installSucceeded()`` returns False. The exception
                message carries the error string returned by the DLL.
        """
        lib = load_dll(dll_name)

        archive_path = Path(archive_path)
        if not archive_path.exists():
            raise FileNotFoundError(f"Archive file not found: {archive_path}")

        install_path = Path(install_path)
        if not install_path.exists():
            raise FileNotFoundError(f"Mod file not found: {install_path}")

        archive_encoded = str(archive_path).encode("utf-8")
        install_encoded = str(install_path).encode("utf-8")
        json_encoded = json_path.encode("utf-8") if json_path else b""

        text = _call_owned_string(
            lib,
            lib.installWithConfig,
            ctypes.c_char_p(archive_encoded),
            ctypes.c_char_p(install_encoded),
            ctypes.c_char_p(json_encoded))

        # The C-API uses the same const char* channel for both success
        # (returns the install path) and failure (returns an error message);
        # installSucceeded() is the authoritative side channel. Raise on
        # failure so the caller can roll back instead of writing meta.ini
        # and refreshing as if everything was fine.
        if not lib.installSucceeded():
            logger.error(f"[install] installWithConfig reported failure: {text}")
            qDebug(f"[install] installWithConfig reported failure: {text}")
            raise InstallError(text)

        return text

    # ------------------------------------------------------------------------------
    # Plugin Metadata and UI Methods
    # ------------------------------------------------------------------------------
    def name(self) -> str:
        """Return the internal name of the plugin."""
        return "Install FOMODs"

    def localizedName(self) -> str:
        """Return the localized name of the plugin for display purposes."""
        return self.tr("Install FOMODs")

    def author(self) -> str:
        """Return the plugin authors."""
        return "MaskPlague & Griffin"

    def description(self):
        """Return a brief description of the plugin functionality."""
        return self.tr("Allows manual selection of multiple archives for sequential installation.")

    def version(self) -> mobase.VersionInfo:
        """Return the plugin version information."""
        return mobase.VersionInfo(1, 1, 0, mobase.ReleaseType.ALPHA)

    def settings(self):
        """
        Return the list of configurable settings for the plugin.

        In this case, it remembers the last opened path used for installations.
        """
        return [
            mobase.PluginSetting("LastPath", self.tr("Last opened path for installing."), "downloads"),
        ]

    def displayName(self):
        """Return the display name of the plugin."""
        return self.tr("Install FOMODs")

    def tooltip(self):
        """Return an empty tooltip text (can be customized if needed)."""
        return self.tr("")

    def icon(self):
        """Return a default icon for the plugin."""
        return QIcon()

    def display(self):
        """
        Open a file dialog for the user to select mod archive files.

        Saves the chosen directory to the plugin settings and initiates the installation queue.
        """
        self._queue = QFileDialog.getOpenFileNames(
            self._parentWidget,
            "Open File",
            self.downloadLocation,
            "Mod Archives (*.001 *.7z *.fomod *.zip *.rar)",
        )[0]
        if len(self._queue) > 0:
            # Update the last used download location based on the first selected file.
            pathGet = self._queue[0]
            self.downloadLocation = os.path.split(os.path.abspath(pathGet))[0]
            self._organizer.setPluginSetting(self.name(), "LastPath", self.downloadLocation)

        self._installQueue()
        return

    def getFiles(self):
        """This method is not used but required by the interface."""
        return

    def tr(self, text):
        """Helper function for translating plugin text."""
        return QCoreApplication.translate("Install FOMODs", text)

    # ------------------------------------------------------------------------------
    # Internal Utility Methods
    # ------------------------------------------------------------------------------
    def _log(self, string):
        """
        Debug logging helper.

        If debug mode is enabled, prints a log message with an incrementing counter.
        """
        if self.debug:
            print("Install Multiple Mods log" + str(self.num) + ": " + string)
            self.num += 1

    def _configure_mod_logger(self, mod_dir: Path):
        """
        Reconfigure the logger to write into a mod-specific log file.

        This method creates (or reuses) a 'salma-install.log' file in the mod directory,
        and sets up a new file handler so that subsequent logs are directed there.

        Args:
            mod_dir (Path): The directory of the mod being installed.
        """
        mod_log_file = mod_dir / "mo_salma.log"
        mod_log_file.parent.mkdir(parents=True, exist_ok=True)
        # Remove all current handlers.
        for handler in logger.handlers[:]:
            logger.removeHandler(handler)
        # Create a new file handler for the mod log file.
        mod_handler = logging.FileHandler(str(mod_log_file))
        mod_handler.setFormatter(formatter)
        logger.addHandler(mod_handler)
        logger.info("Logger reconfigured for mod directory: " + str(mod_dir))

    def _close_logger(self):
        """
        Close and flush all logger handlers.

        This method is used to ensure that all log files are properly written and closed
        after a mod installation is complete.
        """
        for handler in logger.handlers[:]:
            handler.flush()
            handler.close()
            logger.removeHandler(handler)
        logger.debug("Logger handlers closed.")

    def _try_save_fomod_choices(self, archive_path: str, mod_dir: str):
        """
        After a successful installation, try to infer and save FOMOD choices.

        Only runs if the archive had a FOMOD and no choices file exists yet.
        Embeds an identifying metadata block so future ``_find_fomod_json``
        lookups can match by stable identifier instead of filename heuristics.
        """
        mod_name = Path(mod_dir).name
        output_dir = _get_fomod_output_dir(self._organizer)
        choices_file = output_dir / f"{mod_name}.json"
        if choices_file.exists():
            return

        try:
            result = infer_fomod_selections(archive_path, mod_dir)
            if result and result.strip():
                # Validate it's real JSON with steps
                parsed = json.loads(result)
                if "steps" in parsed and len(parsed["steps"]) > 0:
                    _inject_choice_metadata(parsed, archive_path, mod_name)
                    choices_file.write_text(json.dumps(parsed, indent=2),
                                            encoding="utf-8")
                    logger.info(f"Saved FOMOD choices to {choices_file}")
        except Exception as e:
            logger.warning(f"Failed to infer FOMOD choices: {e}")

    def _installQueue(self):
        """
        Process the installation queue.

        Pops archives one at a time. For FOMOD archives with pre-scanned choices
        the salma DLL drives installation; on failure the partial mod folder is
        rolled back via ``shutil.rmtree`` so MO2 does not show a corrupt entry.
        Archives without choices fall through to MO2's built-in installer.
        ``_close_logger()`` always runs in a ``finally`` block so file handlers
        do not leak across iterations. Emits a ``Queue done: N ok, M failed``
        summary when the queue drains.
        """
        if not self.finished or not self._queue:
            return

        success_count = 0
        fail_count = 0

        while self._queue:
            self.finished = False
            archive_path = self._queue.pop(0)
            base_name = os.path.basename(archive_path)
            # Remove file extensions to derive a clean mod name.
            while '.' in base_name:
                base_name = os.path.splitext(base_name)[0]
            # Remove any leading digits or underscores/hyphens.
            base_name = re.sub(r'^\d+[-_]', '', base_name)

            json_path = _find_fomod_json(self._organizer, base_name, archive_path)

            if json_path:
                # FOMOD with pre-scanned choices -- use salma DLL
                mod = self._organizer.createMod(base_name)
                mod_dir = Path(mod.absolutePath())
                self._configure_mod_logger(mod_dir)

                msg = f"[install] Using FOMOD choices: {json_path}"
                logger.info(msg)
                qDebug(msg)

                msg = f"[install] Installing: {base_name}"
                logger.info(msg)
                qDebug(msg)

                try:
                    self.install(archive_path, mod.absolutePath(), json_path)
                    _write_mod_meta_ini(mod_dir, archive_path, base_name)
                    self._organizer.refresh()
                    self._try_save_fomod_choices(archive_path, str(mod_dir))
                    success_count += 1
                    msg = f"[install] Finished: {base_name}"
                    logger.info(msg)
                    qDebug(msg)
                except InstallError as e:
                    fail_count += 1
                    msg = f"[install] Failed: {base_name}: {e}"
                    logger.error(msg)
                    qDebug(msg)
                    # Roll back the partial mod folder so MO2 does not show
                    # the failed install as a real mod. rmtree + refresh is
                    # synchronous and predictable; IModList.removeMod can be
                    # async and prompt the user on some MO2 versions.
                    shutil.rmtree(mod_dir, ignore_errors=True)
                    self._organizer.refresh()
                finally:
                    self._close_logger()
            else:
                # No FOMOD data -- delegate to MO2's built-in installer
                qDebug(f"[install] No FOMOD choices found, using MO2 installer: {base_name}")
                self._manager.installArchive(GuessedString(base_name), archive_path)

            time.sleep(0.2)

        self.finished = True

        if success_count or fail_count:
            msg = f"[install] Queue done: {success_count} ok, {fail_count} failed"
            logger.info(msg)
            qDebug(msg)

        return


# ------------------------------------------------------------------------------
# Plugin Class: ScanFomodChoices
# ------------------------------------------------------------------------------
class ScanFomodChoices(mobase.IPluginTool):
    """
    Scans all installed mods and infers FOMOD selections, saving them as JSON files.

    For each mod that has a source archive containing a FOMOD, this plugin calls the
    C++ inference engine to compare installed files against the FOMOD XML structure
    and determine which options were selected during installation.
    """

    def __init__(self):
        super(ScanFomodChoices, self).__init__()
        self._parentWidget = None
        self._organizer = None

    def init(self, organiser=mobase.IOrganizer, manager=mobase.IInstallationManager):
        self._organizer = organiser
        return True

    def name(self) -> str:
        return "Scan FOMODs"

    def localizedName(self) -> str:
        return self.tr("Scan FOMODs")

    def author(self) -> str:
        return "MaskPlague & Griffin"

    def description(self):
        return self.tr("Scans all installed mods to infer and save FOMOD installation choices.")

    def version(self) -> mobase.VersionInfo:
        return mobase.VersionInfo(1, 1, 0, mobase.ReleaseType.ALPHA)

    def settings(self):
        return []

    def displayName(self):
        return self.tr("Scan FOMODs")

    def tooltip(self):
        return self.tr("Infer FOMOD selections for all installed mods")

    def icon(self):
        return QIcon()

    def tr(self, text):
        return QCoreApplication.translate("Scan FOMODs", text)

    def display(self):
        """
        Scan all installed mods and infer FOMOD selections.

        For each mod:
        1. Read meta.ini to find the source archive (installationFile)
        2. Check if the archive exists and skip if fomod_choices.json already exists
        3. Call inferFomodSelections() via the DLL
        4. Save valid results to <mod_folder>/fomod_choices.json
        """
        mods_dir = self._organizer.modsPath()
        downloads_dir = self._organizer.downloadsPath()
        output_dir = _get_fomod_output_dir(self._organizer)

        def _log(msg):
            logger.info(msg)
            qDebug(msg)

        _log(f"[infer] Starting scan in: {mods_dir}")
        _log(f"[infer] Downloads dir: {downloads_dir}")
        _log(f"[infer] Output dir: {output_dir}")

        scanned = 0
        inferred = 0
        skipped = 0
        no_archive = 0
        no_fomod = 0
        errors = 0

        mods_path = Path(mods_dir)
        if not mods_path.exists():
            _log(f"[infer] Mods directory not found: {mods_dir}")
            return

        # Collect mod directories for progress tracking
        mod_folders = sorted(
            [d for d in mods_path.iterdir() if d.is_dir()]
        )
        total = len(mod_folders)
        _log(f"[infer] Found {total} mod folders")
        status_dot_column = 112

        def _status_line(index, name, status, detail=""):
            label = f"[infer] [{index+1}/{total}] {name}"
            dots = "." * max(4, status_dot_column - len(label))
            suffix = f" {detail}" if detail else ""
            return f"{label} {dots} {status}{suffix}"

        # Progress dialog
        progress = QProgressDialog(
            "Scanning mods for FOMOD choices...", "Cancel", 0, total,
            self._parentWidget)
        progress.setWindowTitle("Scan FOMODs")
        progress.setWindowModality(Qt.WindowModality.WindowModal)
        progress.setMinimumDuration(0)
        progress.setValue(0)

        for i, mod_folder in enumerate(mod_folders):
            if progress.wasCanceled():
                _log("[infer] Cancelled by user")
                break

            mod_name = mod_folder.name
            progress.setLabelText(f"[{i+1}/{total}] {mod_name}")
            progress.setValue(i)
            QCoreApplication.processEvents()

            choices_file = output_dir / f"{mod_name}.json"

            # Skip if choices already exist
            if choices_file.exists():
                skipped += 1
                _log(_status_line(i, mod_name, "SKIP", "(exists)"))
                continue

            # Find the source archive
            archive_path = get_archive_for_mod(str(mod_folder))
            if not archive_path:
                no_archive += 1
                _log(_status_line(i, mod_name, "SKIP", "(no archive)"))
                continue

            # Resolve via the shared C-API helper so the plugin and the
            # dashboard agree on which archive a mod folder maps to. The
            # helper falls back to a downloads-dir lookup for older DLLs
            # that lack the export.
            resolved = resolve_mod_archive(archive_path, str(mod_folder), str(mods_dir))
            if not resolved:
                no_archive += 1
                _log(_status_line(i, mod_name, "SKIP", "(archive missing)"))
                continue
            archive = Path(resolved)

            scanned += 1
            _log(f"[infer] [{i+1}/{total}]   Archive: {archive}")

            try:
                start = time.time()
                # Run DLL call in a background thread so the UI stays responsive.
                # ctypes releases the GIL during the call, so processEvents() runs freely.
                _infer_result = [None]
                _infer_error = [None]
                def _infer_worker():
                    try:
                        _infer_result[0] = infer_fomod_selections(str(archive), str(mod_folder))
                    except Exception as ex:
                        _infer_error[0] = ex
                thread = threading.Thread(target=_infer_worker, daemon=True)
                thread.start()
                while thread.is_alive():
                    QCoreApplication.processEvents()
                    if progress.wasCanceled():
                        thread.join()  # let current mod finish, then break outer loop
                        break
                    thread.join(timeout=0.016)
                if _infer_error[0]:
                    raise _infer_error[0]
                result = _infer_result[0]
                elapsed = time.time() - start

                if result and result.strip():
                    parsed = json.loads(result)
                    if "steps" in parsed and len(parsed["steps"]) > 0:
                        choices_file.write_text(result, encoding="utf-8")
                        inferred += 1
                        _log(_status_line(
                            i, mod_name, "INFERRED",
                            f"({elapsed:.1f}s): {len(parsed['steps'])} steps saved"))
                        _log(f"[infer]   Created: {choices_file}")
                    else:
                        no_fomod += 1
                        _log(_status_line(i, mod_name, "NO STEPS",
                                          f"({elapsed:.1f}s): no FOMOD steps"))
                else:
                    no_fomod += 1
                    _log(_status_line(i, mod_name, "NOT FOMOD",
                                      f"({elapsed:.1f}s): no FOMOD data"))
            except Exception as e:
                errors += 1
                _log(_status_line(i, mod_name, "ERROR", f"({e})"))

        progress.setValue(total)

        summary = (
            f"FOMOD Choices Scan Complete\n\n"
            f"Total mod folders: {total}\n"
            f"Archives processed: {scanned}\n"
            f"Choices inferred: {inferred}\n"
            f"No FOMOD: {no_fomod}\n"
            f"Already had choices: {skipped}\n"
            f"No archive found: {no_archive}\n"
            f"Errors: {errors}"
        )
        _log(f"[infer] {summary}")

        QMessageBox.information(
            self._parentWidget,
            "Scan FOMODs",
            summary
        )

        # Refresh MO2 so it sees the output mod folder
        self._organizer.refresh()


# ------------------------------------------------------------------------------
# Plugin Factory Function
# ------------------------------------------------------------------------------
def createPlugins():
    """
    Factory function to create instances of all plugin classes.

    This is used by the mod organizer to load the plugins.
    """
    return [InstallMods(), ScanFomodChoices()]
