use whatlang::{Lang, detect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LanguageDecision {
    pub is_english: bool,
    pub confidence: f64,
    pub reliable: bool,
}

pub fn detect_english(text: &str, min_confidence: f64) -> LanguageDecision {
    let Some(info) = detect(text) else {
        return LanguageDecision {
            is_english: false,
            confidence: 0.0,
            reliable: false,
        };
    };

    let confidence = info.confidence();
    LanguageDecision {
        is_english: info.lang() == Lang::Eng && info.is_reliable() && confidence >= min_confidence,
        confidence,
        reliable: info.is_reliable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reliable_english() {
        let text = "This article is written in plain English for readers who want to learn about search engines. The page explains how a crawler visits websites, saves documents, follows links, and prepares useful text for indexing. It uses common English words, complete sentences, and enough surrounding context for a language detector to recognize the language with confidence.";
        let decision = detect_english(text, 0.50);
        assert!(decision.is_english);
    }

    #[test]
    fn rejects_non_english() {
        let text = "Este es un texto escrito en espanol sobre busqueda, indices y paginas web. Contiene suficientes palabras para detectar que no esta escrito en ingles.";
        let decision = detect_english(text, 0.50);
        assert!(!decision.is_english);
    }
}
