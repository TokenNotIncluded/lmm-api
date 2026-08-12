/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
export type AssistantMessageSafetyResult = {
  content: string
  redacted: boolean
}

const assistantSensitivePatterns = [
  /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi,
  /\b(?:api[\s_-]?key|access[\s_-]?token|refresh[\s_-]?token|password|passcode|credential)\s*(?:is|:|=)\s*[^\s,;]+/gi,
  /\bauthorization\s*:\s*bearer\s+[^\s,;]+/gi,
  /\bbearer\s+[A-Za-z0-9._~+\-/]+=*/g,
  /\bsk-[A-Za-z0-9_-]{12,}\b/g,
  /\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g,
  /(?<![\w])(?:\+\d{1,3}[\s.-]?)?(?:\(\d{2,4}\)|\d{2,4})[\s.-]?\d{3,4}[\s.-]?\d{4}(?!\w)/g,
] as const

export function redactAssistantMessageForDisplay(
  content: string,
  replacement: string
): AssistantMessageSafetyResult {
  let redacted = false
  let safeContent = content
  for (const pattern of assistantSensitivePatterns) {
    safeContent = safeContent.replace(pattern, () => {
      redacted = true
      return replacement
    })
  }
  return { content: safeContent, redacted }
}
