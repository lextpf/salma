/*!
 * @brief defines values shared by installation and inference.
 * @author Alex (https://github.com/lextpf)
 *
 * file operations carry priority and document order. dependency contexts carry normalized
 * installed-state inputs.
 */

use std::collections::HashSet;

/**
 * @enum FileOpType
 * @brief discriminator for file against folder copy operations.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileOpType {
    #[default]
    File,
    Folder,
}

/**
 * @struct FileOperation
 * @brief a single queued file or folder copy operation.
 * @author Alex (https://github.com/lextpf)
 *
 * `priority` controls overwrite order (higher wins) and `document_order` breaks ties by enqueue
 * position, but the two executors read them differently:
 * [`crate::fomod_service::execute_file_operations`] sorts by `(priority, document_order)`, while
 * `FileOperations::execute` sorts by `priority` alone and uses stable sorting to preserve
 * insertion order.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperation {
    pub op_type: FileOpType,
    /**
     * @brief source path under the extracted archive root.
     * @author Alex (https://github.com/lextpf)
     */
    pub source: String,
    /**
     * @brief destination path under the mod directory.
     * @author Alex (https://github.com/lextpf)
     *
     * same convention as `source`: OS-native at the base, forward-slash in the FOMOD-derived tail,
     * never normalized.
     */
    pub destination: String,
    /**
     * @brief FOMOD priority attribute (MO2 default: 0).
     * @author Alex (https://github.com/lextpf)
     */
    pub priority: i32,
    /**
     * @brief enqueue counter, used as the priority tiebreaker.
     * @author Alex (https://github.com/lextpf)
     *
     * not strictly XML byte-position.
     */
    pub document_order: i32,
}

/**
 * @struct InstallResult
 * @brief outcome of a FOMOD install replay attempt.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallResult {
    /**
     * @brief whether installation completed without error.
     * @author Alex (https://github.com/lextpf)
     */
    pub success: bool,
    pub mod_path: String,
    /**
     * @brief error message if success is false.
     * @author Alex (https://github.com/lextpf)
     */
    pub error: String,
}

/**
 * @enum PluginType
 * @brief FOMOD plugin type descriptor.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PluginType {
    Required,
    Recommended,
    #[default]
    Optional,
    NotUsable,
    CouldBeUsable,
}

/**
 * @struct FomodDependencyContext
 * @brief external state passed to the FOMOD dependency evaluator.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FomodDependencyContext {
    pub game_path: String,
    /**
     * @brief files present in the mod directory.
     * @author Alex (https://github.com/lextpf)
     *
     * every entry must already be in `utils::normalize_path` form, that is lowercase with forward
     * slashes.
     */
    pub installed_files: HashSet<String>,
    pub installed_plugins: HashSet<String>,
    pub installed_fomods: HashSet<String>,
    pub game_version: String,
    pub archive_root: String,
}
