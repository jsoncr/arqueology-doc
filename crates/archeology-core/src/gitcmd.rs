//! Invocación segura del ejecutable `git` sobre repositorios no confiables.
//!
//! La configuración local de un repositorio (`.git/config`) puede definir
//! programas que git ejecuta incluso en comandos de solo lectura: el monitor
//! de archivos (`core.fsmonitor`, que `git status` lanza) o hooks. Todas las
//! llamadas de archeology-doc a git pasan por [`git_command`], que los
//! desactiva con `-c` (tiene prioridad sobre la configuración del repo).

use std::path::Path;
use std::process::Command;

/// Dispositivo nulo como directorio de hooks: ningún hook se encuentra.
#[cfg(windows)]
const NULL_HOOKS: &str = "core.hooksPath=NUL";
#[cfg(not(windows))]
const NULL_HOOKS: &str = "core.hooksPath=/dev/null";

/// Argumentos `-c` que neutralizan la ejecución de programas configurados
/// en el repositorio. Van antes del subcomando.
pub fn safe_git_args() -> [&'static str; 6] {
    [
        "-c",
        "core.fsmonitor=false",
        "-c",
        NULL_HOOKS,
        "-c",
        "core.quotePath=false",
    ]
}

/// `git` en `dir` con la configuración segura de [`safe_git_args`] y sin
/// locks opcionales (`git status` no reescribe el índice).
pub fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(safe_git_args())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(dir);
    cmd
}

/// Hash completo de `HEAD` en `dir` (None sin git, sin repositorio o sin commits).
pub fn head_commit(dir: &Path) -> Option<String> {
    let out = git_command(dir)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!hash.is_empty()).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_args_disable_fsmonitor_and_hooks() {
        let cmd = git_command(Path::new("."));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"core.fsmonitor=false".to_string()));
        assert!(args.iter().any(|a| a.starts_with("core.hooksPath=")));
    }
}
