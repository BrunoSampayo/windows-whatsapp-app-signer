use std::{fs, io, path::PathBuf};

pub const MAX_PROFILES: usize = 100;
pub const MAX_NAME: usize = 60;

#[derive(Clone)]
pub struct Profiles {
    pub names: Vec<String>,
    pub active: usize,
    pub paused: bool,
}

impl Profiles {
    pub fn load() -> io::Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(path) {
            Ok(contents) => {
                let mut lines = contents.lines();
                let active = lines.next().and_then(|line| line.parse().ok()).unwrap_or(0);
                let paused = lines.next() == Some("pausado");
                let names: Vec<String> = lines.take(MAX_PROFILES).map(str::to_owned).collect();
                Ok(Self {
                    active: active.min(names.len().saturating_sub(1)),
                    names,
                    paused,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self {
                names: Vec::new(),
                active: 0,
                paused: false,
            }),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::path()?;
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let temporary = path.with_extension("tmp");
        let mode = if self.paused { "pausado" } else { "ativo" };
        fs::write(
            &temporary,
            format!("{}\n{mode}\n{}", self.active, self.names.join("\n")),
        )?;
        fs::rename(temporary, path)
    }

    pub fn signing_name(&self) -> Option<&str> {
        if self.paused {
            return None;
        }
        self.names.get(self.active).map(String::as_str)
    }

    pub fn validate_name(&self, name: &str, editing: Option<usize>) -> Result<(), &'static str> {
        if name.is_empty()
            || name.encode_utf16().count() > MAX_NAME
            || name.chars().any(char::is_control)
        {
            return Err("Digite um nome de até 60 caracteres, em uma única linha.");
        }
        let comparable = name.to_lowercase();
        if self
            .names
            .iter()
            .enumerate()
            .any(|(index, other)| Some(index) != editing && other.to_lowercase() == comparable)
        {
            return Err("Esse nome já está cadastrado.");
        }
        Ok(())
    }

    fn path() -> io::Result<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::other("A pasta LOCALAPPDATA não está disponível."))?;
        Ok(PathBuf::from(base)
            .join("AssinadorWhatsApp")
            .join("perfis.txt"))
    }
}

pub fn prefix_for_draft(name: &str, draft: &str) -> Option<String> {
    let prefix = format!("*{name}:*");
    let draft =
        draft.trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '\u{200b}' | '\u{feff}'));
    if draft.is_empty() || draft == prefix || has_signature_line(draft, &prefix) {
        return None;
    }
    Some(prefix)
}

pub fn has_signature_line(draft: &str, header: &str) -> bool {
    draft
        .strip_prefix(header)
        .is_some_and(|rest| rest.starts_with(['\r', '\n']))
}

#[cfg(test)]
mod tests {
    use super::prefix_for_draft;

    #[test]
    fn skips_empty_and_already_signed_drafts() {
        for draft in [
            "",
            " \n\u{200b}",
            "*João:*\nOlá",
            "*João:*\r\nOlá",
            "*João:*",
        ] {
            assert_eq!(prefix_for_draft("João", draft), None);
        }
        assert_eq!(
            prefix_for_draft("João", "Olá\nTudo bem?").as_deref(),
            Some("*João:*")
        );
    }
}
