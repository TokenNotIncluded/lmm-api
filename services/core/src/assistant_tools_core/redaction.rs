//! Port of extensions/internal/modules/assistant/redaction (AGPL-3.0-or-later),
//! adapted from fixed main 6efcb6fb model/assistant_history.go.
use regex::{Captures, Regex};
use std::{net::IpAddr, sync::OnceLock};
const FIELD: &str = r"cookie|set-cookie|session(?:[_ -]?id)?|csrf(?:[_ -]?token)?|password|passwd|pwd|api[ _-]?key|access[ _-]?token|refresh[ _-]?token|id[ _-]?token|client[ _-]?secret|secret|token|bearer|authorization|proxy-authorization|密碼|密码|密钥|令牌";
fn go_pattern(source: &str) -> String {
    let mut out = String::new();
    let mut chars = source.chars();
    let mut class = false;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('b') if !class => out.push_str(r"(?-u:\b)"),
                Some('d') => out.push_str(if class { "0-9" } else { "[0-9]" }),
                Some('w') => out.push_str(if class { "A-Za-z0-9_" } else { "[A-Za-z0-9_]" }),
                Some('s') => out.push_str(if class { r"\t\n\f\r " } else { r"[\t\n\f\r ]" }),
                Some(next) => {
                    out.push('\\');
                    out.push(next)
                }
                None => out.push('\\'),
            }
        } else {
            if ch == '[' {
                class = true
            } else if ch == ']' {
                class = false
            }
            out.push(ch)
        }
    }
    out
}
fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(||{let sources:Vec<(String,&'static str)>=vec![
(format!(r#"(?i)(["'](?:{FIELD})["'][\t \r\n]*[:=：][\t \r\n]*)(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')"#),r#"${1}"[REDACTED]""#),
(r"(?i)\b(cookie|set-cookie|authorization|proxy-authorization)[\t ]*[:=：][\t ]*[^\r\n]*".into(),"$1: [REDACTED]"),
(r"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s?#@]+@".into(),"${1}[REDACTED]@"),
(r"(?i)\blmm[ksp]_[a-z0-9_-]{12,}\b".into(),"[REDACTED_CORE_CREDENTIAL]"),
(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)".into(),"[REDACTED_PRIVATE_KEY]"),
(r"(?i)([?&](?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|client[_-]?secret|token|password|passwd|secret)=)[^&#\s]+".into(),"$1[REDACTED]"),
(r"(?i)\b(cookie|set-cookie|session(?:[_ -]?id)?|csrf(?:[_ -]?token)?)\s*[:=：]\s*[^\s;,]+".into(),"$1: [REDACTED]"),
(r"(?i)\bbearer\s+[a-z0-9._~+/-]{6,}=*".into(),"Bearer [REDACTED_TOKEN]"),
(r"(?i)\b(password|passwd|pwd|api[ _-]?key|access[ _-]?token|refresh[ _-]?token|bearer|authorization|密碼|密码|密钥|令牌)\s*[:=：]\s*[^\s,;]+".into(),"$1: [REDACTED]"),
(r"(?i)\[(model_providers|providers)\.[^\]\r\n]*(?:sk|rk|pk|ak)[_-][a-z0-9._~+/-]{20,}[^\]\r\n]*\]".into(),"[$1.REDACTED_API_KEY]"),
(r"(?i)\b(?:sk|rk|pk|ak|tok|token|key|secret)[_-][a-z0-9._~+/-]{8,}\b".into(),"[REDACTED_API_KEY]"),
(r"\beyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\b".into(),"[REDACTED_TOKEN]"),
(r"(?i)\b[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+\b".into(),"[REDACTED_EMAIL]"),
(r"(^|[^\w])((?:\+?86[\s-]?)?1[3-9]\d{9}|\+\d{1,3}(?:[\s.-]?\d{2,4}){2,4})([^\w]|$)".into(),"$1[REDACTED_PHONE]$3"),
(r"\b(?:\d{1,3}\.){3}\d{1,3}\b".into(),"[REDACTED_IP]"),
];sources.into_iter().map(|(p,r)|(Regex::new(&go_pattern(&p)).expect("constant reviewed redaction regex"),r)).collect()})
}
fn luhn(digits: &[u8]) -> bool {
    let mut sum = 0;
    let mut double = false;
    for byte in digits.iter().rev() {
        let mut n = u32::from(byte - b'0');
        if double {
            n *= 2;
            if n > 9 {
                n -= 9
            }
        }
        sum += n;
        double = !double;
    }
    sum % 10 == 0
}
pub fn content(input: &str) -> String {
    let mut s = input.trim().to_owned();
    for (re, r) in patterns() {
        s = re.replace_all(&s, *r).into_owned()
    }
    static IPV6: OnceLock<Regex> = OnceLock::new();
    let ipv6 = IPV6.get_or_init(|| Regex::new(r"[0-9A-Fa-f:]{2,39}").unwrap());
    s = ipv6
        .replace_all(&s, |c: &Captures| {
            let v = &c[0];
            if v.contains(':') && v.parse::<IpAddr>().is_ok() {
                "[REDACTED_IP]".to_owned()
            } else {
                v.to_owned()
            }
        })
        .into_owned();
    static CARD: OnceLock<Regex> = OnceLock::new();
    let card =
        CARD.get_or_init(|| Regex::new(r"(^|[^0-9])([0-9][0-9 -]{11,22}[0-9])([^0-9]|$)").unwrap());
    card.replace_all(&s, |c: &Captures| {
        let digits: Vec<u8> = c[2].bytes().filter(u8::is_ascii_digit).collect();
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            format!("{}[REDACTED_CARD]{}", &c[1], &c[3])
        } else {
            c[0].to_owned()
        }
    })
    .into_owned()
}
#[cfg(test)]
mod tests {
    #[test]
    fn matches_original_go_golden() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("redaction-golden.json")).unwrap();
        for row in rows {
            assert_eq!(
                super::content(row["input"].as_str().unwrap()),
                row["expected"].as_str().unwrap(),
                "index {}",
                row["index"]
            );
        }
    }
}
