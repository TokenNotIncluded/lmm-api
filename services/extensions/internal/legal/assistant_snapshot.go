package legal

import (
	"errors"
	"strings"
)

// PublishedSnapshot is the same immutable, operator-published text served by
// ServeHTTP. It never reads a draft, fetches a URL or permits a policy mutation.
type PublishedSnapshot struct {
	Document            string
	RequestedLanguage   string
	SourceLanguage      string
	Text                string
	Revision            string
	RequestedConfigured bool
	SourceConfigured    bool
}

func (h *Handler) Snapshot(name, language string) (PublishedSnapshot, error) {
	if h == nil || !knownDocument(name) {
		return PublishedSnapshot{}, errors.New("invalid published policy document")
	}
	language = normalizeLanguage(language)
	if language != "" && !languagePattern.MatchString(language) {
		return PublishedSnapshot{}, errors.New("invalid policy language")
	}
	translations := h.documents[name]
	requested, configured := translations[language]
	selected := language
	document, found := requested, configured
	if !found {
		if base, _, regional := strings.Cut(language, "-"); regional {
			selected = base
			document, found = translations[selected]
		}
	}
	if !found {
		selected = h.defaultLanguage
		document, found = translations[selected]
	}
	return PublishedSnapshot{Document: name, RequestedLanguage: language, SourceLanguage: selected, Text: document.text, Revision: document.revision, RequestedConfigured: configured && requested.text != "", SourceConfigured: found && document.text != ""}, nil
}
