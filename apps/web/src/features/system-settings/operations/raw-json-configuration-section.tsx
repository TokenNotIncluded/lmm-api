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
import { Download, FileUp, RefreshCw, Sparkles } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { JsonCodeEditor } from '@/components/json-code-editor'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { requestAssistantOpen } from '@/features/assistant/assistant-events'

import { updateSystemOptions, validateSystemOptions } from '../api'
import { SettingsPageActionsPortal } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useSystemOptions } from '../hooks/use-system-options'

type RawJsonDescriptor = {
  key: string
  label: string
  example: string
}

// This is an explicit allowlist: the raw editor must never expose secrets or
// arbitrary option keys. The server still performs the authoritative checks.
const RAW_JSON_DESCRIPTORS: RawJsonDescriptor[] = [
  {
    key: 'group_ratio_setting.group_warnings',
    label: 'Group warnings',
    example:
      '{\n  "free": {\n    "enabled": true,\n    "message": "Community-operated group. Do not send secrets.",\n    "mode": "modal",\n    "confirmations": 3\n  }\n}',
  },
  {
    key: 'GroupRatio',
    label: 'Group ratios',
    example: '{\n  "default": 1,\n  "premium": 1.2\n}',
  },
  {
    key: 'GroupGroupRatio',
    label: 'Inter-group ratios',
    example: '{\n  "premium": {\n    "default": 1\n  }\n}',
  },
  {
    key: 'TopupGroupRatio',
    label: 'Top-up group ratios',
    example: '{\n  "default": 1,\n  "premium": 1.1\n}',
  },
  {
    key: 'UserUsableGroups',
    label: 'Selectable groups',
    example:
      '{\n  "default": "Standard access",\n  "premium": "Premium access"\n}',
  },
  {
    key: 'AutoGroups',
    label: 'Auto group order',
    example: '[\n  "default",\n  "premium"\n]',
  },
  {
    key: 'group_ratio_setting.group_special_usable_group',
    label: 'Special usable groups',
    example:
      '{\n  "premium": {\n    "+:default": "Standard access",\n    "-:legacy": ""\n  }\n}',
  },
  {
    key: 'AssistantReviewGroupPolicies',
    label: 'AI review group policies',
    example:
      '{\n  "premium": {\n    "probability": 1,\n    "intensity": "high"\n  }\n}',
  },
  {
    key: 'AssistantSkillFiles',
    label: 'AI skill files',
    example:
      '[\n  {\n    "path": "skills/example/SKILL.md",\n    "content": "# Example\\n\\nDescribe the skill here.",\n    "enabled": true\n  }\n]',
  },
  {
    key: 'global.thinking_model_blacklist',
    label: 'Thinking model blacklist',
    example: '[\n  "model-id"\n]',
  },
  {
    key: 'global.chat_completions_to_responses_policy',
    label: 'Chat to Responses policy',
    example: '{\n  "default": "auto"\n}',
  },
  {
    key: 'gemini.safety_settings',
    label: 'Gemini safety settings',
    example: '{\n  "default": "BLOCK_MEDIUM_AND_ABOVE"\n}',
  },
  {
    key: 'gemini.version_settings',
    label: 'Gemini version settings',
    example: '{\n  "default": "v1beta"\n}',
  },
  {
    key: 'gemini.supported_imagine_models',
    label: 'Gemini Imagine models',
    example: '[\n  "gemini-2.5-flash-image"\n]',
  },
  {
    key: 'claude.model_headers_settings',
    label: 'Claude model headers',
    example:
      '{\n  "model-id": {\n    "anthropic-version": "2023-06-01"\n  }\n}',
  },
  {
    key: 'claude.default_max_tokens',
    label: 'Claude default max tokens',
    example: '{\n  "model-id": 4096\n}',
  },
  {
    key: 'billing_setting.billing_mode',
    label: 'Billing modes',
    example: '{\n  "model-id": "ratio"\n}',
  },
  {
    key: 'billing_setting.billing_expr',
    label: 'Billing expressions',
    example: '{\n  "model-id": "input + output"\n}',
  },
  {
    key: 'tool_price_setting.prices',
    label: 'Tool prices',
    example: '{\n  "web_search": 0.001\n}',
  },
  {
    key: 'channel_affinity_setting.rules',
    label: 'Channel affinity rules',
    example:
      '[\n  {\n    "group": "default",\n    "channel_ids": [1, 2]\n  }\n]',
  },
  {
    key: 'AdvancedSecurityRules',
    label: 'Advanced security rules',
    example: '{\n  "version": 1,\n  "rules": []\n}',
  },
  {
    key: 'dynamic_pricing_setting.channel_costs',
    label: 'Dynamic pricing channel costs',
    example: '{\n  "12": 0.5,\n  "34": 1.2\n}',
  },
  {
    key: 'dynamic_pricing_setting.per_model',
    label: 'Dynamic pricing model overrides',
    example:
      '{\n  "model-id": {\n    "target_tpm": 100000,\n    "target_rpm": 60\n  }\n}',
  },
  {
    key: 'console_setting.api_info',
    label: 'API information',
    example:
      '[\n  {\n    "name": "OpenAI compatible",\n    "url": "https://api.example.com/v1"\n  }\n]',
  },
  {
    key: 'console_setting.announcements',
    label: 'Announcements',
    example:
      '[\n  {\n    "content": "Maintenance window",\n    "type": "warning",\n    "publishDate": "2026-01-01T00:00:00Z"\n  }\n]',
  },
  {
    key: 'console_setting.faq',
    label: 'FAQ',
    example:
      '[\n  {\n    "question": "How do I create a key?",\n    "answer": "Open API Keys and confirm the group."\n  }\n]',
  },
  {
    key: 'payment_setting.amount_options',
    label: 'Top-up amount options',
    example: '[\n  5,\n  10,\n  20\n]',
  },
  {
    key: 'payment_setting.amount_discount',
    label: 'Top-up discounts',
    example: '{\n  "100": 0.95,\n  "500": 0.85\n}',
  },
  {
    key: 'PayMethods',
    label: 'Payment methods',
    example:
      '[\n  {\n    "name": "Example",\n    "type": "example",\n    "enabled": true\n  }\n]',
  },
  {
    key: 'CreemProducts',
    label: 'Creem products',
    example: '[\n  {\n    "id": "product_xxx",\n    "name": "Starter"\n  }\n]',
  },
  {
    key: 'WaffoPayMethods',
    label: 'Waffo payment methods',
    example: '[\n  {\n    "name": "Example",\n    "type": "example"\n  }\n]',
  },
]

const descriptorMap = new Map(
  RAW_JSON_DESCRIPTORS.map((descriptor) => [descriptor.key, descriptor])
)

function formatJson(value: string) {
  if (!value.trim()) return ''
  try {
    return JSON.stringify(JSON.parse(value), null, 2)
  } catch {
    return value
  }
}

function parseImport(value: string): string {
  return JSON.stringify(JSON.parse(value), null, 2)
}

function downloadJson(filename: string, value: string) {
  const blob = new Blob([value], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  URL.revokeObjectURL(url)
}

export function RawJsonConfigurationSection() {
  const { t } = useTranslation()
  const optionsQuery = useSystemOptions()
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [selectedKey, setSelectedKey] = useState(
    RAW_JSON_DESCRIPTORS[0]?.key ?? ''
  )
  const [editorValue, setEditorValue] = useState('')
  const [baselineValue, setBaselineValue] = useState('')
  const [validationMessage, setValidationMessage] = useState<string | null>(
    null
  )
  const [isValidating, setIsValidating] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const descriptor = descriptorMap.get(selectedKey)
  const availableDescriptors = useMemo(
    () =>
      RAW_JSON_DESCRIPTORS.filter((item) =>
        optionsQuery.data?.data?.some((option) => option.key === item.key)
      ),
    [optionsQuery.data?.data]
  )

  useEffect(() => {
    if (
      availableDescriptors.length > 0 &&
      !availableDescriptors.some((item) => item.key === selectedKey)
    ) {
      setSelectedKey(availableDescriptors[0].key)
    }
  }, [availableDescriptors, selectedKey])

  useEffect(() => {
    const raw =
      optionsQuery.data?.data?.find((option) => option.key === selectedKey)
        ?.value ?? ''
    const formatted = formatJson(raw)
    setEditorValue(formatted)
    setBaselineValue(formatted)
    setValidationMessage(null)
  }, [descriptor?.example, optionsQuery.data?.data, selectedKey])

  const validate = async () => {
    if (!selectedKey || !editorValue.trim()) {
      setValidationMessage(t('A JSON value is required.'))
      return false
    }
    try {
      JSON.parse(editorValue)
    } catch {
      setValidationMessage(t('Invalid JSON. Please fix the syntax first.'))
      return false
    }
    setIsValidating(true)
    try {
      const response = await validateSystemOptions({
        [selectedKey]: editorValue,
      })
      if (!response.success) {
        setValidationMessage(response.message || t('Configuration is invalid.'))
        return false
      }
      setValidationMessage(t('Configuration validated.'))
      return true
    } catch (error) {
      setValidationMessage(
        error instanceof Error
          ? error.message
          : t('Configuration check failed.')
      )
      return false
    } finally {
      setIsValidating(false)
    }
  }

  const save = async () => {
    if (!(await validate())) return
    setIsSaving(true)
    try {
      const response = await updateSystemOptions({
        [selectedKey]: editorValue,
      })
      if (!response.success) {
        throw new Error(response.message || t('Failed to update setting'))
      }
      setBaselineValue(editorValue)
      toast.success(t('Setting updated successfully'))
      await optionsQuery.refetch()
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : t('Failed to update setting')
      )
    } finally {
      setIsSaving(false)
    }
  }

  const handleImport = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    void file.text().then((text) => {
      try {
        setEditorValue(parseImport(text))
        setValidationMessage(null)
      } catch {
        setValidationMessage(t('The imported file is not valid JSON.'))
      }
    })
  }

  const askAssistant = () => {
    const safeValue = editorValue.slice(0, 12_000)
    requestAssistantOpen(
      'service',
      `请帮我编辑管理员配置 ${selectedKey}。先读取当前服务端配置并解释字段，再提出最小变更预览；未经我确认不要写入。当前编辑器内容如下：\n\n${safeValue}`
    )
  }

  const dirty = editorValue !== baselineValue

  return (
    <SettingsSection title={t('Raw JSON Configuration')}>
      <div className='space-y-4'>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Edit one safe-listed JSON setting at a time. Imports are checked locally and by the server before any write.'
          )}
        </p>
        <div className='flex flex-wrap items-end gap-3'>
          <div className='min-w-56 flex-1 space-y-2'>
            <Label>{t('Configuration key')}</Label>
            <Select
              value={selectedKey}
              onValueChange={(value) => {
                if (value) setSelectedKey(value)
              }}
            >
              <SelectTrigger>
                <SelectValue placeholder={t('Select')} />
              </SelectTrigger>
              <SelectContent>
                {availableDescriptors.map((item) => (
                  <SelectItem key={item.key} value={item.key}>
                    {t(item.label)} · {item.key}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <input
            ref={fileInputRef}
            type='file'
            accept='application/json,.json'
            className='hidden'
            onChange={handleImport}
          />
          <Button
            type='button'
            variant='outline'
            onClick={() => fileInputRef.current?.click()}
          >
            <FileUp data-icon='inline-start' />
            {t('Import JSON')}
          </Button>
          <Button
            type='button'
            variant='outline'
            onClick={() => downloadJson(`${selectedKey}.json`, editorValue)}
            disabled={!editorValue.trim()}
          >
            <Download data-icon='inline-start' />
            {t('Export JSON')}
          </Button>
          <Button type='button' variant='outline' onClick={askAssistant}>
            <Sparkles data-icon='inline-start' />
            {t('Ask AI to edit')}
          </Button>
        </div>
        <JsonCodeEditor
          value={editorValue}
          onChange={(value) => {
            setEditorValue(value)
            setValidationMessage(null)
          }}
          example={descriptor?.example}
          disabled={optionsQuery.isLoading || !descriptor}
          heightClassName='h-[28rem] min-h-[28rem] max-h-[28rem]'
          ariaLabel={t('JSON')}
        />
        {validationMessage && (
          <Alert
            variant={
              validationMessage === t('Configuration validated.')
                ? 'default'
                : 'destructive'
            }
          >
            <RefreshCw />
            <AlertTitle>{t('Configuration check')}</AlertTitle>
            <AlertDescription>{validationMessage}</AlertDescription>
          </Alert>
        )}
        <SettingsPageActionsPortal>
          <Button
            type='button'
            variant='outline'
            onClick={() => void validate()}
            disabled={isValidating || isSaving || !dirty}
          >
            {isValidating ? t('Checking...') : t('Validate configuration')}
          </Button>
          <Button
            type='button'
            onClick={() => void save()}
            disabled={isValidating || isSaving || !dirty}
          >
            {isSaving ? t('Saving...') : t('Save Changes')}
          </Button>
        </SettingsPageActionsPortal>
      </div>
    </SettingsSection>
  )
}
