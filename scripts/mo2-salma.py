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
import sys
import ctypes
import json
import configparser

from pathlib import Path
from mobase import GuessedString

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
# The log file is created in a 'logs' subdirectory in the current working directory.
default_log_file = Path.cwd() / r"logs\mo_salma.log"
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

def load_dll(dll_name="mo2-salma.dll"):
    """Load and cache the DLL. Returns the ctypes library handle."""
    if dll_name not in _dll_cache:
        dll_path = find_dll(dll_name)
        logger.info(f"Loading DLL: {dll_path}")
        _dll_cache[dll_name] = ctypes.CDLL(str(dll_path))
    return _dll_cache[dll_name]


def infer_fomod_selections(archive_path: str, mod_path: str) -> str:
    """Call the DLL's inferFomodSelections function and return the JSON string."""
    lib = load_dll()
    lib.inferFomodSelections.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    lib.inferFomodSelections.restype = ctypes.c_char_p

    result = lib.inferFomodSelections(
        archive_path.encode("utf-8"),
        mod_path.encode("utf-8"))

    if result:
        return result.decode("utf-8")
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


def _find_fomod_json(organizer, mod_name: str) -> str:
    """Find a FOMOD choices JSON for the given mod name.

    Tries exact match first, then checks if any existing JSON stem is a
    prefix of mod_name (e.g. 'A Cat's Life' matches
    'A Cat's Life-37250-2-0-1655543026').  Returns the path as a string,
    or empty string if nothing matches.
    """
    output_dir = _get_fomod_output_dir(organizer)
    exact = output_dir / f"{mod_name}.json"
    if exact.exists():
        return str(exact)

    # Fuzzy: longest-stem-first so the most specific match wins
    candidates = sorted(output_dir.glob("*.json"),
                        key=lambda p: len(p.stem), reverse=True)
    lower_name = mod_name.lower()
    for candidate in candidates:
        if lower_name.startswith(candidate.stem.lower()):
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
        content += f"1\\fileid={modid}\n"

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

        This method loads the DLL, sets up the function signature for the log callback,
        and registers the callback function with the DLL. It also saves the callback
        reference to prevent it from being garbage-collected.
        """
        try:
            # Create a callback instance with the defined function signature.
            c_callback = CALLBACK_TYPE(InstallMods.log_callback)
            lib = load_dll()
            # Define the expected argument and return types for the setLogCallback function.
            lib.setLogCallback.argtypes = [CALLBACK_TYPE]
            lib.setLogCallback.restype = None
            # Register the callback with the DLL.
            lib.setLogCallback(c_callback)
            # Retain a reference to the callback to avoid garbage collection.
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
            str: The output from the DLL function, decoded to a Python string.

        Raises:
            FileNotFoundError: If either the archive or installation path does not exist.
        """
        lib = load_dll(dll_name)

        lib.installWithConfig.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
        lib.installWithConfig.restype = ctypes.c_char_p

        archive_path = Path(archive_path)
        if not archive_path.exists():
            raise FileNotFoundError(f"Archive file not found: {archive_path}")

        install_path = Path(install_path)
        if not install_path.exists():
            raise FileNotFoundError(f"Mod file not found: {install_path}")

        archive_encoded = str(archive_path).encode("utf-8")
        install_encoded = str(install_path).encode("utf-8")
        json_encoded = json_path.encode("utf-8")
        result = lib.installWithConfig(
            ctypes.c_char_p(archive_encoded),
            ctypes.c_char_p(install_encoded),
            ctypes.c_char_p(json_encoded))

        return result.decode("utf-8")

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
        return mobase.VersionInfo(1, 0, 0, mobase.ReleaseType.ALPHA)

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
                    choices_file.write_text(result, encoding="utf-8")
                    logger.info(f"Saved FOMOD choices to {choices_file}")
        except Exception as e:
            logger.warning(f"Failed to infer FOMOD choices: {e}")

    def _installQueue(self):
        """
        Process the installation queue.

        If there are mod archive files in the queue and no installation is running,
        this method pops the next archive, prepares its mod name, creates a mod entry,
        reconfigures the logger for that mod, and calls the installation method.
        After installation, it refreshes the organizer and proceeds with the next item.
        """
        if self.finished and len(self._queue) != 0:
            self.finished = False
            archive_path = self._queue.pop(0)
            base_name = os.path.basename(archive_path)
            # Remove file extensions to derive a clean mod name.
            while '.' in base_name:
                base_name = os.path.splitext(base_name)[0]
            # Remove any leading digits or underscores/hyphens.
            base_name = re.sub(r'^\d+[-_]', '', base_name)

            json_path = _find_fomod_json(self._organizer, base_name)

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
                self.install(archive_path, mod.absolutePath(), json_path)
                _write_mod_meta_ini(mod_dir, archive_path, base_name)
                self._organizer.refresh()
                msg = f"[install] Finished: {base_name}"
                logger.info(msg)
                qDebug(msg)

                # Auto-save FOMOD choices after successful install
                self._try_save_fomod_choices(archive_path, str(mod_dir))

                self._close_logger()
            else:
                # No FOMOD data -- delegate to MO2's built-in installer
                qDebug(f"[install] No FOMOD choices found, using MO2 installer: {base_name}")
                self._manager.installArchive(GuessedString(base_name), archive_path)
            time.sleep(0.2)
            self.finished = True
            self._installQueue()

        elif len(self._queue) == 0:
            self.finished = True

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
        return mobase.VersionInfo(1, 0, 0, mobase.ReleaseType.ALPHA)

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

            # Resolve relative paths against downloads directory
            archive = Path(archive_path)
            if not archive.is_absolute():
                archive = Path(downloads_dir) / archive

            if not archive.exists():
                no_archive += 1
                _log(_status_line(i, mod_name, "SKIP", "(archive missing)"))
                continue

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
