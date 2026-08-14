package common

import (
	"encoding/base64"
	"strings"
	"testing"
)

func TestParseEmailRecipientsRejectsHeaderInjection(t *testing.T) {
	_, _, err := parseEmailRecipients("victim@example.com\r\nBcc: attacker@example.com")
	if err == nil {
		t.Fatal("recipient containing an injected header was accepted")
	}
}

func TestParseEmailRecipientsBuildsCanonicalHeaderAndEnvelope(t *testing.T) {
	header, recipients, err := parseEmailRecipients("first@example.com; second@example.com")
	if err != nil {
		t.Fatal(err)
	}
	if header != "<first@example.com>, <second@example.com>" {
		t.Fatalf("header = %q", header)
	}
	if len(recipients) != 2 || recipients[0] != "first@example.com" || recipients[1] != "second@example.com" {
		t.Fatalf("recipients = %#v", recipients)
	}
}

func TestBuildEmailMessageRejectsHeaderBreaks(t *testing.T) {
	for name, value := range map[string]string{
		"to":         "victim@example.com\r\nBcc: attacker@example.com",
		"from":       "LMM\r\nBcc: attacker@example.com <sender@example.com>",
		"subject":    "=?UTF-8?B?c3ViamVjdA==?=\r\nBcc: attacker@example.com",
		"date":       "Wed, 12 Aug 2026 12:00:00 +0800\r\nBcc: attacker@example.com",
		"message-id": "<id@example.com>\r\nBcc: attacker@example.com",
	} {
		values := map[string]string{
			"to":         "victim@example.com",
			"from":       "LMM <sender@example.com>",
			"subject":    "=?UTF-8?B?c3ViamVjdA==?=",
			"date":       "Wed, 12 Aug 2026 12:00:00 +0800",
			"message-id": "<id@example.com>",
		}
		values[name] = value
		if _, err := buildEmailMessage(values["to"], values["from"], values["subject"], values["date"], values["message-id"], "<p>合法邮件</p>"); err == nil {
			t.Fatalf("header %s accepted CRLF injection", name)
		}
	}
}

func TestBuildEmailMessageKeepsHTMLBodyAfterHeaders(t *testing.T) {
	content := "<p>合法邮件</p>\r\nBcc: attacker@example.com\r\n<img src=x onerror=alert(1)>"
	message, err := buildEmailMessage(
		"victim@example.com",
		"LMM <sender@example.com>",
		"=?UTF-8?B?c3ViamVjdA==?=",
		"Wed, 12 Aug 2026 12:00:00 +0800",
		"<id@example.com>",
		content,
	)
	if err != nil {
		t.Fatal(err)
	}

	parts := strings.SplitN(string(message), "\r\n\r\n", 2)
	if len(parts) != 2 {
		t.Fatalf("message has no MIME header/body separator: %q", message)
	}
	if strings.Contains(parts[0], "Bcc:") {
		t.Fatalf("body content became a message header: %q", parts[0])
	}
	encodedBody := strings.ReplaceAll(parts[1], "\r\n", "")
	decodedBody, err := base64.StdEncoding.DecodeString(encodedBody)
	if err != nil {
		t.Fatalf("encoded MIME body is invalid: %v", err)
	}
	if string(decodedBody) != content {
		t.Fatalf("HTML body was not preserved after MIME decoding: %q", decodedBody)
	}
}

func TestSanitizeEmailHTMLPreservesMarkupAndRemovesUnsafeHTML(t *testing.T) {
	content, err := sanitizeEmailHTML(
		`<p>合法</p><a href="https://example.com/path?a=1&amp;b=2" title="说明">链接</a>` +
			`<script>alert(1)</script><img src=x onerror="alert(2)">` +
			`<a href="javascript:alert(3)">危险链接</a>`,
	)
	if err != nil {
		t.Fatal(err)
	}
	for _, fragment := range []string{
		"<p>合法</p>",
		`<a href="https://example.com/path?a=1&amp;b=2" title="说明">链接</a>`,
	} {
		if !strings.Contains(content, fragment) {
			t.Fatalf("safe HTML was not preserved %q: %s", fragment, content)
		}
	}
	for _, unsafe := range []string{"<script", "<img", "onerror", "javascript:"} {
		if strings.Contains(strings.ToLower(content), unsafe) {
			t.Fatalf("unsafe HTML fragment survived sanitization %q: %s", unsafe, content)
		}
	}
}
