"""
Plugin: Install Mods & Scan FOMOD Choices
Authors: MaskPlague & Griffin

Two MO2 tools built on the salma engine DLL (`mo2-salma.dll`):

  - Install FOMODs: installs selected archives one after another, applying the
    FOMOD choices saved for each mod when there are any, and handing anything
    else to MO2's own installer.
  - Scan FOMODs: infers the FOMOD selections of every installed mod by comparing
    its files against the FOMOD structure in its archive, and saves one JSON per
    mod.

The module locates and loads the DLL, registers a log callback so engine lines
reach MO2's log window, and owns the output folder both tools write into.

Host requirements: `mobase` and PyQt6 are imported at module scope, so this
module only imports inside MO2. `scripts/smoke_plugin.py` stubs both to exercise
the loader outside MO2.

DLL lifetime: `load_dll` caches the ctypes handle in `_dll_cache` for the life of
the process, so replacing `mo2-salma.dll` on disk has no effect until MO2
restarts.
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
    """Raised by :meth:`InstallMods.install` when the engine DLL reports failure.

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
# Handlers are cleared first so this module owns exactly one: a handler left
# over from an earlier import would duplicate every line.
logger = logging.getLogger(__name__)
logger.setLevel(logging.DEBUG)

if logger.hasHandlers():
    logger.handlers.clear()

# The log file sits next to this plugin file, under <MO2 plugins>/logs/, so its
# location does not depend on the working directory MO2 was launched from.
# InstallMods._configure_mod_logger swaps this handler out per mod install.
default_log_file = Path(__file__).parent / "logs" / "mo_salma.log"
default_log_file.parent.mkdir(parents=True, exist_ok=True)
default_handler = logging.FileHandler(str(default_log_file))
formatter = logging.Formatter('%(asctime)s - %(levelname)s - %(message)s')
default_handler.setFormatter(formatter)
logger.addHandler(default_handler)

logger.debug("Logger configured successfully.")

# ------------------------------------------------------------------------------
# Callback Function Definition
# ------------------------------------------------------------------------------
# The C ABI signature the DLL expects: void(const char*), receiving one
# already-formatted UTF-8 log line.
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

# Major version of the DLL ABI this plugin is written against. Bumped only on
# ABI-incompatible changes: a removed export or a changed signature. Adding an
# export is a minor bump, which is why the newer exports below are gated on
# hasattr instead. A mismatch refuses to use the DLL rather than risk silent ABI
# drift.
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
    """Locate the required DLL. Shared by all plugin classes.

    Search order, first existing file wins:
      1. `<this file's directory>/salma` - where deploy.bat puts the DLL.
      2. The current working directory.
      3. `<cwd>/dlls/salma`.
      4. `<this file's directory>`.
      5. Every existing entry of %PATH%, in order.

    Returns a resolved absolute Path. Raises FileNotFoundError when no
    candidate holds `dll_name`. Directories that cannot be probed (permission
    errors) are skipped rather than raising.

    This is the order MO2 actually uses, so it is the authority.
    `scripts/common.py::find_dll` runs a different, shorter order for the test
    harness, and only candidate 1 has a counterpart there.
    """
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

    Owned-string returns (install / installWithConfig / inferFomodSelections /
    resolveModArchive) are declared as ``c_void_p`` so ctypes hands back the raw
    heap pointer, which can then go to freeResult once the bytes are copied.
    Declaring them ``c_char_p`` instead lets ctypes auto-decode and silently
    leaks the allocation on every call.
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

    Raises RuntimeError, after logging the same text, when the reported major
    version differs from EXPECTED_API_MAJOR and when the version string is
    empty. Only the leading major component is compared.

    The AttributeError branch below cannot be reached through `load_dll`:
    `_configure_dll` runs first and touches `lib.getApiVersion` while declaring
    argtypes, so a DLL missing that export raises a bare AttributeError there
    and the caller never sees this guidance. Treat the branch as defensive.
    Moving the presence check to the top of `_configure_dll` would make it
    reachable again.
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

    The cache is keyed by `dll_name` and lives for the whole process, so the
    DLL is located, loaded and version-checked once per MO2 session. Replacing
    the file on disk has no effect until MO2 restarts. Windows also keeps the
    loaded file locked, so a deploy over a running MO2 fails.
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
    """Invoke a DLL function returning a C string the DLL allocated.

    Only ``freeResult`` may release that pointer. Decodes UTF-8 and always calls
    it, including when decoding throws, so the allocation cannot leak. Returns
    "" for a null pointer.
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

    An absolute installationFile that exists is returned before the DLL is
    loaded at all. Delegates to the DLL's ``resolveModArchive`` (added in salma
    1.1.0) when the loaded DLL exports it. Without that export the fallback
    below tries ``$SALMA_DOWNLOADS_PATH/<installationFile>`` only, so it keeps
    an older deploy working only where that variable is set (setup.bat sets it;
    MO2 does not). Returns "" when nothing resolves.
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

    # Fallback for DLLs older than 1.1.0: the downloads directory only.
    # Deliberately narrower than the C-API path, which also tries mods-dir
    # siblings, so it resolves fewer archives rather than guessing wrong ones.
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
    """Return the central FOMOD output folder, creating it when absent.

    Also writes a stub meta.ini for the containing mod folder, which is what
    makes MO2 list "Salma FOMODs Output" as a mod instead of ignoring it.
    """
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
    """Add the `metadata` block to a parsed FOMOD-choices JSON.

    The block lets `_find_fomod_json` match by (modid, fileid) or by
    (size, mtime) instead of comparing filenames. Its schema must stay aligned
    with `inject_choice_metadata` in Mo2FomodController.cpp: both writers fill
    the same output directory, and `_find_fomod_json` reads whatever it finds
    there.

    Mutates `parsed` in place and returns None. An unreadable archive is not an
    error: `archive_size` and `archive_mtime` are written as 0, which never
    matches a live archive.
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
         against the metadata block. Size in bytes, exact; mtime in whole
         POSIX seconds, matched within one second. Cheap fingerprint for
         non-Nexus mods.
      3. Exact filename match: `<mod_name>.json`. Backward compatible with
         JSONs scanned before the metadata block existed.
      4. Deprecated stem-prefix fuzzy match, longest stem first. Logs a
         warning.
    Returns the path as a string, or empty string if nothing matches.

    All four priorities read `<mods>/Salma FOMODs Output/fomods/*.json`.
    Priorities 1 and 2 need a JSON that carries a `metadata` block, and not
    every writer produces one. The dashboard's scan endpoint
    (Mo2FomodController.cpp) and `InstallMods._try_save_fomod_choices` both
    inject it; `ScanFomodChoices.display()` writes the engine's raw output and
    does not. Re-running the plugin's own "Scan FOMODs" tool therefore cannot
    lift a mod off priority 4. Delete the JSON and re-scan from the dashboard to
    get a metadata block.
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
    Plugin tool for installing mod archives, one after another.

    Implements MO2's IPluginTool. It drives the salma engine DLL for archives
    that have saved FOMOD choices, keeps the installation queue, and redirects
    logging into each mod folder while that mod installs.
    """

    def __init__(self):
        super(InstallMods, self).__init__()
        self._parentWidget = None  # Reference to the parent GUI widget (if needed)
        self._log_callback = None  # Store the log callback instance to prevent garbage collection

    def init(self, organiser=mobase.IOrganizer, manager=mobase.IInstallationManager):
        """
        Take the organizer and installation-manager handles MO2 supplies.

        Sets up the instance state, logs the Python environment for debugging,
        and registers the log callback with the DLL. Returns True; MO2 treats a
        False return as a failed plugin.
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

        # Register the Python log callback with the engine DLL.
        self.setLog()
        return True

    @staticmethod
    def log_callback(message: bytes):
        """
        Receive one already-formatted log line from the engine DLL.

        Decodes the C string, writes it to this module's logger, and forwards it
        to MO2's application log window through qDebug.

        The engine calls this synchronously, on whichever thread made the DLL
        call, including the worker thread ScanFomodChoices.display() uses. While
        a callback is registered the engine sends its lines here instead of
        writing logs/salma.log; registering a null callback restores file
        logging.
        """
        text = message.decode("utf-8")
        logger.info(text)
        qDebug(text.encode("ascii", "replace").decode("ascii"))

    def setLog(self):
        """
        Register the Python log callback with the engine DLL.

        Loads the DLL, which configures argtypes and restypes on first load,
        then registers the callback. The callback object is kept on ``self``:
        if it is garbage-collected the engine calls a dangling pointer.
        Failures are logged, not raised, so a missing DLL does not break the
        tool menu.
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
        Install one archive through the DLL's ``installWithConfig``.

        With a json_path the engine applies those selections against
        ModuleConfig.xml. With an empty json_path it derives
        ``<archive stem>.json`` next to the archive and uses that when the file
        exists, and only otherwise installs with the FOMOD's own defaults. A
        non-empty json_path is used verbatim, with no sidecar fallback; if it
        does not exist the install proceeds with the FOMOD's defaults instead of
        failing.

        Args:
            archive_path (str): The mod archive.
            install_path (str): Target directory. Must already exist.
            json_path (str): Optional FOMOD choices JSON.
            dll_name (str): DLL to load, keyed into the module-level cache.

        Returns:
            str: The install path the engine reports on success.

        Raises:
            FileNotFoundError: If the archive or the install path is absent.
            InstallError: If ``installSucceeded()`` returns False. The message
                carries the error string the DLL returned.
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

        # The C API returns the same const char* for success (the install path)
        # and failure (an error message), so installSucceeded() is the only
        # authority. Raise on failure, or the caller writes meta.ini and
        # refreshes MO2 as though a broken install had worked.
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
        Point the module logger at a log file inside the mod being installed.

        Opens (or appends to) `mo_salma.log` in `mod_dir` and makes it the only
        handler: every existing handler is removed first, so the default
        `<plugin dir>/logs/mo_salma.log` receives nothing until a handler is
        added again. `_close_logger` ends that state.

        The file lands inside the installed mod, so `mo_salma.log` has to stay
        in IGNORED_FILES in scripts/common.py. Without it every round-trip diff
        reports this log as an extra file in every mod.

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
        Flush, close and detach every logger handler.

        Run this after each install so the mod-local log file is written out and
        released before the next mod reconfigures the logger.
        """
        for handler in logger.handlers[:]:
            handler.flush()
            handler.close()
            logger.removeHandler(handler)
        logger.debug("Logger handlers closed.")

    def _try_save_fomod_choices(self, archive_path: str, mod_dir: str):
        """
        After a successful install, infer the mod's FOMOD choices and save them.

        Does nothing when a choices file for this mod name already exists, and
        writes nothing when the archive holds no FOMOD. The saved JSON carries a
        metadata block, so later ``_find_fomod_json`` lookups match on a stable
        identifier rather than on the filename. Inference failures are logged as
        warnings: the install has already succeeded and must not be undone.
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
        Process the installation queue, one archive at a time.

        An archive with pre-scanned FOMOD choices is installed through the salma
        DLL; a failure deletes the partial mod folder with ``shutil.rmtree`` so
        MO2 never lists a half-installed mod. An archive without choices goes to
        MO2's built-in installer instead. ``_close_logger()`` runs in a
        ``finally`` block, so mod-local file handlers cannot leak into the next
        iteration. Logs a ``Queue done: N ok, M failed`` summary once the queue
        drains, unless every archive went to MO2's own installer and both counts
        are zero.

        The mod name is the archive basename truncated at its first dot (the
        splitext loop strips every suffix, not only the archive one) with a
        leading ``<digits>-`` or ``<digits>_`` download prefix removed. It
        becomes the MO2 folder name, the ``_find_fomod_json`` lookup key and the
        input to the Nexus id parse, so a version number in the filename
        shortens all three.
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
    Scan every installed mod and save the FOMOD selections inferred for it.

    For each mod whose source archive holds a FOMOD, this tool calls the engine
    DLL's inferFomodSelections export, which compares the installed files
    against the FOMOD structure and recovers the options chosen at install time.
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

        Results go to one central output mod, never into the mod folders:
        `<mods>/Salma FOMODs Output/fomods/<mod_name>.json`, created by
        `_get_fomod_output_dir` together with a stub meta.ini so MO2 lists it.

        For each mod folder under `organizer.modsPath()`, in this order:
        1. Skip if `<output>/<mod_name>.json` already exists. Nothing is
           overwritten, so re-running only fills gaps; delete a file to redo it.
        2. Read meta.ini for the source archive (installationFile) and resolve
           it with `resolve_mod_archive`, which uses the DLL's resolveModArchive
           export where it exists. Skip the mod if either step returns empty.
        3. Call inferFomodSelections() through the DLL on a worker thread, so
           the progress dialog keeps repainting.
        4. Write the result verbatim to `<output>/<mod_name>.json`, but only if
           it parses as JSON and holds at least one entry in "steps".

        Step 4 writes the engine's raw output, which carries no `metadata` block
        (the dashboard scan endpoint adds one). `_find_fomod_json` cannot match
        those files by identifier and falls back to its filename rules; see its
        docstring.

        Cancelling from the progress dialog stops after the mod in flight, and
        work already written stays. Returns None, reports the totals in a
        message box, and refreshes MO2 so the output mod appears in the list.
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
                        # cancel requested: wait out this mod, leave the wait
                        # loop, and let the next iteration's wasCanceled check
                        # end the scan. this mod's result is still parsed and
                        # written below.
                        thread.join()
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
