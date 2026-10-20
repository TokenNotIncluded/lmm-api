package personalization

import "encoding/json"

type definition struct {
	name, description, effect, access string
	schema                            json.RawMessage
}

var definitions = []definition{
	{"get_overview_greeting", "Read the current user's saved overview greeting templates, revision and variables. Defaults are localized by the browser. This never changes a preference.", "read_only", "user", json.RawMessage("{\"additionalProperties\":false,\"properties\":{},\"type\":\"object\"}")},
	{"set_overview_greeting", "Prepare a localized overview greeting change for the current user only. Supported variables: $name, $time, $date, $weekday, $level, $site, $balance; $$ means a literal dollar sign. Empty template resets that locale to its default. The browser must confirm the exact change.", "confirmation", "user", json.RawMessage("{\"additionalProperties\":false,\"properties\":{\"language\":{\"enum\":[\"en\",\"zh\",\"zh-TW\",\"fr\",\"ja\",\"ru\",\"vi\"],\"type\":\"string\"},\"template\":{\"maxLength\":512,\"type\":\"string\"}},\"required\":[\"language\",\"template\"],\"type\":\"object\"}")},
	{"set_conversation_title", "Set a short automatic title for this new conversation. Summarize the user's actual task in 3-8 specific words, in the user's language. Never use a greeting, generic label, secret, or complete sentence.", "server_guarded", "user", json.RawMessage("{\"additionalProperties\":false,\"properties\":{\"title\":{\"maxLength\":60,\"minLength\":2,\"type\":\"string\"}},\"required\":[\"title\"],\"type\":\"object\"}")},
}
