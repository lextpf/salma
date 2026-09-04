"""
@brief provide MO2 install and FOMOD scan tools.
@author Alex (https://github.com/lextpf)

InstallMods applies saved choices through the salma DLL and delegates other
archives to MO2. ScanFomodChoices infers choices from installed files.

the module imports `mobase` and PyQt6 at load time. the cached DLL remains
loaded and locked until MO2 exits.
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
    """
    @class InstallError
    @brief report an install that the engine marks as failed.
    @author Alex (https://github.com/lextpf)

    the message is the text returned by the DLL.
    """

from PyQt6.QtCore import QCoreApplication, Qt, qDebug
from PyQt6.QtGui import QIcon
from PyQt6.QtWidgets import QFileDialog, QMessageBox, QProgressDialog

# clear existing handlers to keep one copy of each line.
logger = logging.getLogger(__name__)
logger.setLevel(logging.DEBUG)

if logger.hasHandlers():
    logger.handlers.clear()

# use a plugin-relative log until InstallMods selects a mod-local log.
default_log_file = Path(__file__).parent / "logs" / "mo_salma.log"
default_log_file.parent.mkdir(parents=True, exist_ok=True)
default_handler = logging.FileHandler(str(default_log_file))
formatter = logging.Formatter('%(asctime)s - %(levelname)s - %(message)s')
default_handler.setFormatter(formatter)
logger.addHandler(default_handler)

logger.debug("Logger configured successfully.")

# the C ABI supplies one formatted UTF-8 line as void(const char*).
CALLBACK_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_char_p)

# reject ABI-incompatible DLLs; optional exports do not change this major.
EXPECTED_API_MAJOR = "1"

def _safe_path_exists(p: str) -> bool:
    try:
        return Path(p).exists()
    except OSError:
        return False


def find_dll(dll_name="mo2-salma.dll"):
    """
    @fn find_dll(dll_name="mo2-salma.dll")
    @brief select the first accessible plugin DLL.
    @author Alex (https://github.com/lextpf)

    ### :material-format-list-numbered: lookup order

    | order | location                     |
    |-------|------------------------------|
    | 1     | plugin directory/salma       |
    | 2     | current directory            |
    | 3     | current directory/dlls/salma |
    | 4     | plugin directory             |
    | 5     | each PATH entry              |

    the result is an absolute Path. inaccessible directories do not stop the
    search. absence raises FileNotFoundError.
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
    """
    @fn _configure_dll(lib)
    @brief preserve ownership of strings allocated by the DLL.
    @author Alex (https://github.com/lextpf)

    owned strings use `c_void_p` and must pass to `freeResult`. static version
    text uses `c_char_p` and must not be freed. `resolveModArchive` is optional.
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

    # leave the optional export unset so resolve_mod_archive can use its fallback.
    if hasattr(lib, "resolveModArchive"):
        lib.resolveModArchive.argtypes = [
            ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
        lib.resolveModArchive.restype = ctypes.c_void_p


def _check_api_version(lib):
    """
    @fn _check_api_version(lib)
    @brief reject missing, empty, or incompatible ABI versions.
    @author Alex (https://github.com/lextpf)

    only the leading major component is compared. failure is logged and raises
    RuntimeError.
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
    """
    @fn load_dll(dll_name="mo2-salma.dll")
    @brief configure and cache one DLL handle per name.
    @author Alex (https://github.com/lextpf)

    ### :material-lock-outline: DLL lifetime

    the cache lasts for the MO2 process. Windows keeps each loaded file locked,
    so replacement requires MO2 to exit.
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
    """
    @fn _call_owned_string(lib, fn, *args) -> str
    @brief copy and release an owned UTF-8 result from the DLL.
    @author Alex (https://github.com/lextpf)

    `freeResult` runs even when decoding fails. a null pointer returns an empty
    string.
    """
    addr = fn(*args)
    if not addr:
        return ""
    try:
        return ctypes.string_at(addr).decode("utf-8")
    finally:
        lib.freeResult(addr)


def infer_fomod_selections(archive_path: str, mod_path: str) -> str:
    lib = load_dll()
    return _call_owned_string(
        lib,
        lib.inferFomodSelections,
        archive_path.encode("utf-8"),
        mod_path.encode("utf-8"))


def resolve_mod_archive(installation_file: str, mod_folder: str, mods_dir: str) -> str:
    """
    @fn resolve_mod_archive(installation_file: str, mod_folder: str, mods_dir: str) -> str
    @brief use the dashboard archive-resolution contract when available.
    @author Alex (https://github.com/lextpf)

    an existing absolute path bypasses the DLL. without `resolveModArchive`,
    only `SALMA_DOWNLOADS_PATH` is searched. failure returns an empty string.
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

    # keep the fallback narrower than the C API to avoid speculative matches.
    downloads_dir = os.environ.get("SALMA_DOWNLOADS_PATH", "")
    if downloads_dir:
        candidate = Path(downloads_dir) / archive
        if candidate.exists():
            return str(candidate)
    return ""


def get_archive_for_mod(mod_path: str) -> str:
    meta_ini = Path(mod_path) / "meta.ini"
    if not meta_ini.exists():
        return ""

    config = configparser.ConfigParser()
    try:
        config.read(str(meta_ini), encoding="utf-8")
    except Exception:
        return ""

    archive = config.get("General", "installationFile", fallback="")
    return archive


_FOMOD_OUTPUT_MOD = "Salma FOMODs Output"


def _get_fomod_output_dir(organizer) -> Path:
    """
    @fn _get_fomod_output_dir(organizer) -> Path
    @brief ensure MO2 can list the central choices directory.
    @author Alex (https://github.com/lextpf)

    a stub meta.ini makes the containing folder visible as an MO2 mod.
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
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(data, dict):
        return {}
    meta = data.get("metadata", {})
    return meta if isinstance(meta, dict) else {}


def _inject_choice_metadata(parsed: dict, archive_path: str, module_name: str) -> None:
    """
    @fn _inject_choice_metadata(parsed: dict, archive_path: str, module_name: str) -> None
    @brief attach stable archive identity to a choices document.
    @author Alex (https://github.com/lextpf)

    the schema must match `inject_choice_metadata` in Mo2FomodController.cpp.
    unreadable archives use zero size and mtime values and cannot match.
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
    """
    @fn _find_fomod_json(organizer, mod_name: str, archive_path: str = "") -> str
    @brief select choices with the strongest available archive identity.
    @author Alex (https://github.com/lextpf)

    ### :material-format-list-numbered: match priority

    | priority | match                    |
    |----------|--------------------------|
    | 1        | Nexus mod id and file id |
    | 2        | archive size and mtime   |
    | 3        | exact JSON filename      |
    | 4        | longest fuzzy stem       |

    size is exact in bytes. mtime uses whole POSIX seconds with a one-second
    tolerance. ScanFomodChoices output has no metadata and uses filename
    matching. failure returns an empty string.
    """
    output_dir = _get_fomod_output_dir(organizer)
    candidates = sorted(output_dir.glob("*.json"),
                        key=lambda p: len(p.stem), reverse=True)

    # prefer Nexus identifiers.
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

    # use the archive fingerprint when identifiers are absent.
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

    # use the generated filename before fuzzy matching.
    exact = output_dir / f"{mod_name}.json"
    if exact.exists():
        return str(exact)

    # keep fuzzy matching as the final fallback.
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
    match = re.match(r'^(.+?)-(\d+)-(.*)-(\d+)$', filename)
    if match:
        return {
            'modid': match.group(2),
            'version': match.group(3).replace('-', '.'),
            'fileid': match.group(4),
        }
    return {}


def _write_mod_meta_ini(mod_dir: Path, archive_path: str, mod_name: str):
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

class InstallMods(mobase.IPluginTool):
    """
    @class InstallMods
    @brief install selected archives through salma or MO2.
    @author Alex (https://github.com/lextpf)

    saved FOMOD choices select the salma engine. other archives use the MO2
    installer. engine failures remove the partial mod directory.
    """

    def __init__(self):
        super(InstallMods, self).__init__()
        self._parentWidget = None
        self._log_callback = None  # keep the callback alive while C code holds its pointer.

    def init(self, organiser=mobase.IOrganizer, manager=mobase.IInstallationManager):
        """
        @fn init(self, organiser=mobase.IOrganizer, manager=mobase.IInstallationManager)
        @brief initialize MO2 handles without failing plugin registration.
        @author Alex (https://github.com/lextpf)

        callback registration failures are logged. the method still returns
        True.
        """
        self.debug = True
        self.num = 0
        self._modList = []
        self.finished = True
        self._organizer = organiser
        self._manager = manager
        self._queue = []
        self.handler = None

        self.downloadLocation = self._organizer.pluginSetting(self.name(), "LastPath")

        logger.info(sys.version)
        logger.info(sys.executable)
        logger.info(os.getcwd())
        logger.info(platform.architecture())

        self.setLog()
        return True

    @staticmethod
    def log_callback(message: bytes):
        """
        @fn log_callback(message: bytes)
        @brief forward one engine line to the file and MO2 logs.
        @author Alex (https://github.com/lextpf)

        the engine invokes the callback synchronously on the calling thread.
        qDebug receives ASCII replacement text.
        """
        text = message.decode("utf-8")
        logger.info(text)
        qDebug(text.encode("ascii", "replace").decode("ascii"))

    def setLog(self):
        """
        @fn setLog(self)
        @brief keep a live callback while the DLL holds its pointer.
        @author Alex (https://github.com/lextpf)

        failures are logged and do not prevent the tool menu from loading.
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
        lib = load_dll(dll_name)

        archive_path = Path(archive_path)
        if not archive_path.exists():
            raise FileNotFoundError(f"Archive file not found: {archive_path}")

        install_path = Path(install_path)
        if not install_path.exists():
            raise FileNotFoundError(f"Mod file not found: {install_path}")

        archive_encoded = str(archive_path).encode("utf-8")
        install_encoded = str(install_path).encode("utf-8")
        # empty selects an archive-side JSON or defaults; nonempty uses only that path.
        json_encoded = json_path.encode("utf-8") if json_path else b""

        text = _call_owned_string(
            lib,
            lib.installWithConfig,
            ctypes.c_char_p(archive_encoded),
            ctypes.c_char_p(install_encoded),
            ctypes.c_char_p(json_encoded))

        # the return text is ambiguous; installSucceeded is the failure authority.
        if not lib.installSucceeded():
            logger.error(f"[install] installWithConfig reported failure: {text}")
            qDebug(f"[install] installWithConfig reported failure: {text}")
            raise InstallError(text)

        return text

    def name(self) -> str:
        return "Install FOMODs"

    def localizedName(self) -> str:
        return self.tr("Install FOMODs")

    def author(self) -> str:
        return "MaskPlague & Griffin"

    def description(self):
        return self.tr("Allows manual selection of multiple archives for sequential installation.")

    def version(self) -> mobase.VersionInfo:
        return mobase.VersionInfo(1, 1, 0, mobase.ReleaseType.ALPHA)

    def settings(self):
        return [
            mobase.PluginSetting("LastPath", self.tr("Last opened path for installing."), "downloads"),
        ]

    def displayName(self):
        return self.tr("Install FOMODs")

    def tooltip(self):
        return self.tr("")

    def icon(self):
        return QIcon()

    def display(self):
        self._queue = QFileDialog.getOpenFileNames(
            self._parentWidget,
            "Open File",
            self.downloadLocation,
            "Mod Archives (*.001 *.7z *.fomod *.zip *.rar)",
        )[0]
        if len(self._queue) > 0:
            pathGet = self._queue[0]
            self.downloadLocation = os.path.split(os.path.abspath(pathGet))[0]
            self._organizer.setPluginSetting(self.name(), "LastPath", self.downloadLocation)

        self._installQueue()
        return

    def getFiles(self):
        return

    def tr(self, text):
        return QCoreApplication.translate("Install FOMODs", text)

    def _log(self, string):
        if self.debug:
            print("Install Multiple Mods log" + str(self.num) + ": " + string)
            self.num += 1

    def _configure_mod_logger(self, mod_dir: Path):
        """
        @fn _configure_mod_logger(self, mod_dir: Path)
        @brief isolate engine logs in the current mod directory.
        @author Alex (https://github.com/lextpf)

        the file is opened for append and becomes the only handler.
        `scripts.common.IGNORED_FILES` must exclude it from install comparisons.
        """
        mod_log_file = mod_dir / "mo_salma.log"
        mod_log_file.parent.mkdir(parents=True, exist_ok=True)
        for handler in logger.handlers[:]:
            logger.removeHandler(handler)
        mod_handler = logging.FileHandler(str(mod_log_file))
        mod_handler.setFormatter(formatter)
        logger.addHandler(mod_handler)
        logger.info("Logger reconfigured for mod directory: " + str(mod_dir))

    def _close_logger(self):
        """
        @fn _close_logger(self)
        @brief release the mod-local log before the next install.
        @author Alex (https://github.com/lextpf)

        every handler is flushed, closed, and detached.
        """
        for handler in logger.handlers[:]:
            handler.flush()
            handler.close()
            logger.removeHandler(handler)
        logger.debug("Logger handlers closed.")

    def _try_save_fomod_choices(self, archive_path: str, mod_dir: str):
        """
        @fn _try_save_fomod_choices(self, archive_path: str, mod_dir: str)
        @brief preserve stable choices after a successful install.
        @author Alex (https://github.com/lextpf)

        existing files are not overwritten. absent FOMOD data writes nothing.
        inference failures warn but do not undo the install.
        """
        mod_name = Path(mod_dir).name
        output_dir = _get_fomod_output_dir(self._organizer)
        choices_file = output_dir / f"{mod_name}.json"
        if choices_file.exists():
            return

        try:
            result = infer_fomod_selections(archive_path, mod_dir)
            if result and result.strip():
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
        @fn _installQueue(self)
        @brief keep sequential installs isolated and recoverable.
        @author Alex (https://github.com/lextpf)

        saved choices use the salma DLL. other archives use the MO2 installer.

        ### :material-shield-check: failure isolation

        an engine failure removes the partial mod directory. each mod-local log
        closes before the next archive.

        ### :material-link-variant: mod names

        the mod name removes all suffixes and one leading numeric download
        prefix. it is also the folder and choices lookup name.
        """
        if not self.finished or not self._queue:
            return

        success_count = 0
        fail_count = 0

        while self._queue:
            self.finished = False
            archive_path = self._queue.pop(0)
            base_name = os.path.basename(archive_path)
            while '.' in base_name:
                base_name = os.path.splitext(base_name)[0]
            base_name = re.sub(r'^\d+[-_]', '', base_name)

            json_path = _find_fomod_json(self._organizer, base_name, archive_path)

            if json_path:
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
                    # remove synchronously; IModList.removeMod can prompt or run async.
                    shutil.rmtree(mod_dir, ignore_errors=True)
                    self._organizer.refresh()
                finally:
                    self._close_logger()
            else:
                qDebug(f"[install] No FOMOD choices found, using MO2 installer: {base_name}")
                self._manager.installArchive(GuessedString(base_name), archive_path)

            time.sleep(0.2)

        self.finished = True

        if success_count or fail_count:
            msg = f"[install] Queue done: {success_count} ok, {fail_count} failed"
            logger.info(msg)
            qDebug(msg)

        return


class ScanFomodChoices(mobase.IPluginTool):
    """
    @class ScanFomodChoices
    @brief recover installed FOMOD selections from source archives.
    @author Alex (https://github.com/lextpf)

    results are stored in the central output mod.
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
        @fn display(self)
        @brief write inferred choices without overwriting existing records.
        @author Alex (https://github.com/lextpf)

        results use `<mods>/Salma FOMODs Output/fomods/<mod_name>.json`.
        missing archives and results without steps are skipped.
        raw engine output has no metadata, so filename matching applies.

        ### :material-timer-outline: cancellation

        inference runs on a worker thread so the progress dialog can repaint.
        cancellation waits for the active inference and keeps completed files.
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

            if choices_file.exists():
                skipped += 1
                _log(_status_line(i, mod_name, "SKIP", "(exists)"))
                continue

            archive_path = get_archive_for_mod(str(mod_folder))
            if not archive_path:
                no_archive += 1
                _log(_status_line(i, mod_name, "SKIP", "(no archive)"))
                continue

            # share the dashboard resolver; its fallback is downloads-only.
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
                # ctypes releases the GIL, so the UI can repaint during inference.
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
                        # finish the active mod; the next iteration ends the scan.
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

        self._organizer.refresh()


def createPlugins():
    """
    @fn createPlugins()
    @brief expose both tools through the MO2 plugin entry point.
    @author Alex (https://github.com/lextpf)

    """
    return [InstallMods(), ScanFomodChoices()]
