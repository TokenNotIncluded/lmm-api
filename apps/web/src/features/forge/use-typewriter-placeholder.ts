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
import { useReducedMotion } from 'motion/react'
import { useEffect, useState } from 'react'

type TypewriterCursor = {
  promptIndex: number
  characterIndex: number
  deleting: boolean
}

const initialCursor: TypewriterCursor = {
  promptIndex: 0,
  characterIndex: 0,
  deleting: false,
}

export function useTypewriterPlaceholder(
  prompts: string[],
  enabled: boolean
): string {
  const reducedMotion = useReducedMotion()
  const normalizedPrompts = prompts
    .map((prompt) => prompt.trim())
    .filter((prompt) => prompt.length > 0)
    .slice(0, 4)
  const promptSignature = normalizedPrompts.join('\u0000')
  const [cursor, setCursor] = useState<TypewriterCursor>(initialCursor)

  useEffect(() => {
    setCursor(initialCursor)
  }, [promptSignature])

  const prompt =
    normalizedPrompts[cursor.promptIndex % normalizedPrompts.length] ?? ''
  const visibleText = prompt.slice(0, cursor.characterIndex)

  useEffect(() => {
    if (!enabled || reducedMotion || !prompt) return

    let delay = cursor.deleting ? 28 : 52
    if (!cursor.deleting && cursor.characterIndex >= prompt.length) {
      delay = 1_500
    } else if (cursor.deleting && cursor.characterIndex === 0) {
      delay = 320
    }

    const timer = window.setTimeout(() => {
      setCursor((current) => {
        if (!current.deleting && current.characterIndex >= prompt.length) {
          return { ...current, deleting: true }
        }
        if (current.deleting && current.characterIndex === 0) {
          return {
            promptIndex: (current.promptIndex + 1) % normalizedPrompts.length,
            characterIndex: 0,
            deleting: false,
          }
        }
        return {
          ...current,
          characterIndex: current.characterIndex + (current.deleting ? -1 : 1),
        }
      })
    }, delay)

    return () => window.clearTimeout(timer)
  }, [
    cursor.characterIndex,
    cursor.deleting,
    enabled,
    normalizedPrompts.length,
    prompt,
    reducedMotion,
  ])

  if (!enabled) return ''
  if (reducedMotion) return normalizedPrompts[0] ?? ''
  return visibleText
}
