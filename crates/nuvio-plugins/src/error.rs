#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error("O manifesto do repositório não é JSON válido.")]
    Json,
    #[error("O manifesto do repositório não tem nome.")]
    EmptyName,
    #[error("O manifesto do repositório não tem versão.")]
    EmptyVersion,
    #[error("O manifesto do repositório não lista nenhum scraper.")]
    NoScrapers,
    #[error("Um scraper não tem identificador ou ficheiro.")]
    IncompleteScraper,
    #[error("Endereço do repositório inválido.")]
    InvalidUrl,
    #[error("Só são aceites endereços http(s).")]
    UnsupportedUrl,
    #[error("O plugin demorou demasiado ou excedeu o limite de trabalho.")]
    Timeout,
    #[error("Falha no motor JavaScript: {0}")]
    Runtime(String),
}

impl From<serde_json::Error> for PluginError {
    fn from(_: serde_json::Error) -> Self {
        Self::Json
    }
}

impl From<url::ParseError> for PluginError {
    fn from(_: url::ParseError) -> Self {
        Self::InvalidUrl
    }
}
