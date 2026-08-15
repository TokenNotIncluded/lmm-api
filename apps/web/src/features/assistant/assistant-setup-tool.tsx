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
import {
  ArrowRight01Icon,
  ComputerTerminal01Icon,
  ExternalLinkIcon,
  Key01Icon,
  LaptopIcon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { CopyButton } from '@/components/copy-button'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

import {
  detectAssistantSetupPlatform,
  getCCSwitchInstallGuide,
  getClaudeInstallCommand,
  getClaudeSessionCommand,
  getCodexAPIKeyCommand,
  getCodexConfig,
  getCodexConfigPath,
  getCodexInstallCommand,
  getOpenAICompatibleClientJSON,
  type AssistantSetupPlatform,
} from './setup-guide'

const CLAUDE_INSTALL_DOCS = 'https://code.claude.com/docs/en/installation'
const CLAUDE_DESKTOP_DOCS = 'https://code.claude.com/docs/en/desktop-quickstart'
const CC_SWITCH_RELEASES = 'https://github.com/farion1231/cc-switch/releases'
const CC_SWITCH_INSTALL_DOCS =
  'https://github.com/farion1231/cc-switch/blob/main/docs/user-manual/en/1-getting-started/1.2-installation.md'
const CC_SWITCH_PROVIDER_DOCS =
  'https://github.com/farion1231/cc-switch/blob/main/docs/user-manual/en/2-providers/2.1-add.md'
const CC_SWITCH_DESKTOP_DOCS =
  'https://github.com/farion1231/cc-switch/blob/main/docs/user-manual/en/2-providers/2.6-claude-desktop.md'
const CHATGPT_DOWNLOAD = 'https://chatgpt.com/download/'
const CHATGPT_WEB = 'https://chatgpt.com/'
const CODEX_DOCS = 'https://developers.openai.com/codex/'
const CODEX_CONFIG_DOCS = 'https://developers.openai.com/codex/config-reference'
const CURSOR_DOWNLOADS = 'https://www.cursor.com/en/downloads'
const OPEN_WEBUI_GUIDE =
  'https://docs.openwebui.com/getting-started/quick-start/connect-a-provider/starting-with-openai-compatible/'
const PLATFORM_LABELS: Record<AssistantSetupPlatform, string> = {
  windows: 'Windows',
  macos: 'macOS',
  linux: 'Linux',
}
type ClientTab =
  | 'claude-code'
  | 'cc-switch'
  | 'claude-desktop'
  | 'chatgpt'
  | 'codex'
  | 'openai-compatible'

type NavigatorWithUserAgentData = Navigator & {
  userAgentData?: { platform?: string }
}

function detectBrowserSetupPlatform(): AssistantSetupPlatform {
  if (typeof navigator === 'undefined') return 'windows'
  const browserNavigator = navigator as NavigatorWithUserAgentData
  return detectAssistantSetupPlatform(
    browserNavigator.userAgentData?.platform,
    browserNavigator.userAgent
  )
}

function CodeSnippet(props: { label: string; value: string }) {
  return (
    <div className='grid gap-1.5'>
      <span className='text-muted-foreground text-xs'>{props.label}</span>
      <div className='bg-background flex items-start gap-2 rounded-lg border p-2.5'>
        <pre className='min-w-0 flex-1 overflow-x-auto font-mono text-[11px] leading-5 whitespace-pre-wrap'>
          {props.value}
        </pre>
        <CopyButton value={props.value} size='sm' />
      </div>
    </div>
  )
}

function ConnectionValue(props: { label: string; value: string }) {
  return (
    <div className='flex items-center justify-between gap-2 border-b py-2 last:border-b-0'>
      <span className='text-muted-foreground text-xs'>{props.label}</span>
      <div className='flex min-w-0 items-center gap-1.5'>
        <code className='truncate text-xs'>{props.value}</code>
        <CopyButton value={props.value} size='sm' />
      </div>
    </div>
  )
}

function OfficialLink(props: { href: string; label: string }) {
  return (
    <Button
      variant='outline'
      size='sm'
      render={<a href={props.href} target='_blank' rel='noopener noreferrer' />}
    >
      {props.label}
      <HugeiconsIcon
        icon={ExternalLinkIcon}
        strokeWidth={2}
        data-icon='inline-end'
        aria-hidden='true'
      />
    </Button>
  )
}

function SetupStep(props: {
  number: number
  title: string
  description: string
}) {
  return (
    <li className='flex items-start gap-2.5'>
      <Badge
        variant='secondary'
        className='mt-0.5 size-5 shrink-0 justify-center rounded-full p-0'
      >
        {props.number}
      </Badge>
      <div className='min-w-0'>
        <p className='text-xs font-medium'>{props.title}</p>
        <p className='text-muted-foreground mt-0.5 text-xs leading-5'>
          {props.description}
        </p>
      </div>
    </li>
  )
}

export function AssistantSetupTool(props: {
  rootUrl: string
  openAIBaseUrl: string
  availableModels: string[]
  modelsLoading?: boolean
  developerAccessGranted: boolean
  onCreateKey: () => void
  onRequestAccess: () => void
}) {
  const { t } = useTranslation()
  const [platform, setPlatform] = useState<AssistantSetupPlatform>(
    detectBrowserSetupPlatform
  )
  const [clientTab, setClientTab] = useState<ClientTab>('claude-code')
  const [selectedModel, setSelectedModel] = useState('')
  const installCommand = getClaudeInstallCommand(platform)
  const ccSwitchInstall = getCCSwitchInstallGuide(platform)
  const codexInstallCommand = getCodexInstallCommand(platform)
  const chatGPTDesktopAvailable = platform !== 'linux'

  if (!props.developerAccessGranted) {
    return (
      <Card size='sm'>
        <CardHeader>
          <CardTitle className='flex items-center gap-2'>
            <HugeiconsIcon
              icon={LaptopIcon}
              className='size-4'
              strokeWidth={2}
              aria-hidden='true'
            />
            {t('Client setup guide')}
          </CardTitle>
          <CardDescription>
            {t(
              'You can install clients while L0 access is under review. API requests become available after L1 approval.'
            )}
          </CardDescription>
        </CardHeader>
        <CardContent className='grid gap-4'>
          <Alert>
            <HugeiconsIcon
              icon={LaptopIcon}
              strokeWidth={2}
              aria-hidden='true'
            />
            <AlertTitle>{t('Ask for L1 access')}</AlertTitle>
            <AlertDescription>
              {t(
                'Connection values and API key creation unlock only after an administrator approves L1.'
              )}
            </AlertDescription>
          </Alert>

          <div className='grid gap-1.5'>
            <span className='text-muted-foreground text-xs'>
              {t('Platform')}
            </span>
            <div className='flex flex-wrap gap-2' aria-label={t('Platform')}>
              {(['windows', 'macos', 'linux'] as const).map((item) => (
                <Button
                  key={item}
                  type='button'
                  size='sm'
                  variant={platform === item ? 'default' : 'outline'}
                  aria-pressed={platform === item}
                  onClick={() => setPlatform(item)}
                >
                  {PLATFORM_LABELS[item]}
                </Button>
              ))}
            </div>
          </div>

          <Tabs
            value={clientTab}
            onValueChange={(value) => setClientTab(value as ClientTab)}
          >
            <TabsList className='flex h-auto w-full flex-wrap justify-start gap-1'>
              <TabsTrigger value='claude-code'>Claude Code</TabsTrigger>
              <TabsTrigger value='cc-switch'>CC Switch</TabsTrigger>
              <TabsTrigger value='claude-desktop'>Claude Desktop</TabsTrigger>
              <TabsTrigger value='chatgpt'>ChatGPT</TabsTrigger>
              <TabsTrigger value='codex'>Codex</TabsTrigger>
            </TabsList>

            <TabsContent value='claude-code' className='mt-3 grid gap-3'>
              <CodeSnippet
                label={t('Install command')}
                value={installCommand}
              />
              <div className='flex flex-wrap gap-2'>
                <OfficialLink
                  href={CLAUDE_INSTALL_DOCS}
                  label={t('Official installation guide')}
                />
                <OfficialLink
                  href={CLAUDE_DESKTOP_DOCS}
                  label='Claude Code Desktop'
                />
              </div>
            </TabsContent>

            <TabsContent value='cc-switch' className='mt-3 grid gap-3'>
              <Alert>
                <AlertTitle>
                  {t('Use official CC Switch downloads only')}
                </AlertTitle>
                <AlertDescription>
                  {t(
                    'CC Switch is free and open source. An installer asking for payment, top-ups, or account credentials is not official.'
                  )}
                </AlertDescription>
              </Alert>
              {ccSwitchInstall.command ? (
                <CodeSnippet
                  label={t('Install CC Switch on {{platform}}', {
                    platform: PLATFORM_LABELS[platform],
                  })}
                  value={ccSwitchInstall.command}
                />
              ) : (
                <div className='bg-muted/40 rounded-lg border p-3 text-xs leading-5'>
                  {t(
                    'Download {{artifact}} from GitHub Releases, open it, and finish the Windows installer.',
                    { artifact: ccSwitchInstall.artifact }
                  )}
                </div>
              )}
              <Alert>
                <AlertTitle>{t('CC Switch one-click import')}</AlertTitle>
                <AlertDescription>
                  {t(
                    "After L1 approval and API key creation, the assistant can build a private ccswitch:// import link for you. It opens CC Switch's confirmation dialog, so you do not need to enter provider fields manually."
                  )}
                </AlertDescription>
              </Alert>
              <div className='flex flex-wrap gap-2'>
                <OfficialLink
                  href={CC_SWITCH_RELEASES}
                  label={t('Open official releases')}
                />
                <OfficialLink
                  href={CC_SWITCH_INSTALL_DOCS}
                  label={t('Installation manual')}
                />
              </div>
            </TabsContent>

            <TabsContent value='claude-desktop' className='mt-3 grid gap-3'>
              {platform === 'linux' ? (
                <Alert>
                  <AlertTitle>
                    {t(
                      'CC Switch Desktop provider setup is not available on Linux'
                    )}
                  </AlertTitle>
                  <AlertDescription>
                    {t(
                      'Claude Desktop for Linux is available in beta, but CC Switch currently writes third-party Desktop profiles only on Windows and macOS. Use Claude Code on Linux for this service.'
                    )}
                  </AlertDescription>
                </Alert>
              ) : null}
              <OfficialLink
                href={CLAUDE_DESKTOP_DOCS}
                label={t('Official Desktop guide')}
              />
            </TabsContent>

            <TabsContent value='chatgpt' className='mt-3 grid gap-3'>
              <Alert>
                <AlertTitle>
                  {t('Official ChatGPT desktop uses OpenAI sign-in')}
                </AlertTitle>
                <AlertDescription>
                  {t(
                    'The official ChatGPT desktop app does not accept this service Base URL and API key as a custom provider. Use CC Switch or an OpenAI-compatible client for this service.'
                  )}
                </AlertDescription>
              </Alert>
              <OfficialLink
                href={chatGPTDesktopAvailable ? CHATGPT_DOWNLOAD : CHATGPT_WEB}
                label={
                  chatGPTDesktopAvailable
                    ? t('Download official ChatGPT')
                    : t('Open ChatGPT in browser')
                }
              />
            </TabsContent>

            <TabsContent value='codex' className='mt-3 grid gap-3'>
              <CodeSnippet
                label={t('Install command')}
                value={codexInstallCommand}
              />
              <OfficialLink
                href={CODEX_DOCS}
                label={t('Official Codex guide')}
              />
            </TabsContent>
          </Tabs>

          <Button type='button' onClick={props.onRequestAccess}>
            {t('Unlock L1 access')}
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              data-icon='inline-end'
              aria-hidden='true'
            />
          </Button>
        </CardContent>
      </Card>
    )
  }

  const model = props.availableModels.includes(selectedModel)
    ? selectedModel
    : (props.availableModels[0] ?? '<MODEL_ID>')
  const sessionCommand = getClaudeSessionCommand(platform, props.rootUrl, model)
  const openAICompatibleConfig = getOpenAICompatibleClientJSON(
    props.openAIBaseUrl,
    model
  )
  const codexAPIKeyCommand = getCodexAPIKeyCommand(platform)
  const codexConfigPath = getCodexConfigPath(platform)
  const codexConfig = getCodexConfig(props.openAIBaseUrl, model)
  let modelOptions: ReactNode = (
    <NativeSelectOption value='<MODEL_ID>'>
      {t('No available models')}
    </NativeSelectOption>
  )
  if (props.modelsLoading) {
    modelOptions = (
      <NativeSelectOption value='<MODEL_ID>'>
        {t('Loading current models...')}
      </NativeSelectOption>
    )
  } else if (props.availableModels.length > 0) {
    modelOptions = props.availableModels.map((item) => (
      <NativeSelectOption key={item} value={item}>
        {item}
      </NativeSelectOption>
    ))
  }

  return (
    <Card size='sm'>
      <CardHeader>
        <CardTitle className='flex items-center gap-2'>
          <HugeiconsIcon
            icon={LaptopIcon}
            className='size-4'
            strokeWidth={2}
            aria-hidden='true'
          />
          {t('Client setup guide')}
        </CardTitle>
        <CardDescription>
          {t(
            'Copy the exact values for this service. API keys stay as placeholders until you create and copy one yourself.'
          )}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className='mb-3 grid gap-1.5'>
          <span className='text-muted-foreground text-xs'>{t('Platform')}</span>
          <div className='flex flex-wrap gap-2' aria-label={t('Platform')}>
            {(['windows', 'macos', 'linux'] as const).map((item) => (
              <Button
                key={item}
                type='button'
                size='sm'
                variant={platform === item ? 'default' : 'outline'}
                aria-pressed={platform === item}
                onClick={() => setPlatform(item)}
              >
                {PLATFORM_LABELS[item]}
              </Button>
            ))}
          </div>
        </div>

        <div className='mb-3 grid gap-1.5'>
          <span className='text-muted-foreground text-xs'>{t('Model ID')}</span>
          <NativeSelect
            value={model}
            disabled={props.modelsLoading || props.availableModels.length === 0}
            onChange={(event) => setSelectedModel(event.target.value)}
            aria-label={t('Model ID')}
          >
            {modelOptions}
          </NativeSelect>
        </div>

        <Tabs
          value={clientTab}
          onValueChange={(value) => setClientTab(value as ClientTab)}
        >
          <TabsList className='flex h-auto w-full flex-wrap justify-start gap-1'>
            <TabsTrigger value='claude-code'>Claude Code</TabsTrigger>
            <TabsTrigger value='cc-switch'>CC Switch</TabsTrigger>
            <TabsTrigger value='claude-desktop'>Claude Desktop</TabsTrigger>
            <TabsTrigger value='chatgpt'>ChatGPT</TabsTrigger>
            <TabsTrigger value='codex'>Codex</TabsTrigger>
            <TabsTrigger value='openai-compatible'>
              {t('OpenAI-compatible clients')}
            </TabsTrigger>
          </TabsList>

          <TabsContent value='claude-code' className='mt-3 grid gap-3'>
            <CodeSnippet label={t('Install command')} value={installCommand} />
            <CodeSnippet
              label={
                platform === 'windows'
                  ? t('PowerShell session configuration')
                  : t('Shell session configuration')
              }
              value={sessionCommand}
            />
            <p className='text-muted-foreground text-xs leading-5'>
              {t(
                'These variables apply to the current terminal session. The API key is sent as a Bearer token and is never embedded by this guide.'
              )}
            </p>
            <div className='flex flex-wrap gap-2'>
              <OfficialLink
                href={CLAUDE_INSTALL_DOCS}
                label={t('Official installation guide')}
              />
              <OfficialLink
                href={CLAUDE_DESKTOP_DOCS}
                label='Claude Code Desktop'
              />
              <Button
                size='sm'
                variant='outline'
                onClick={props.onCreateKey}
                disabled={!props.developerAccessGranted}
              >
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  data-icon='inline-start'
                  aria-hidden='true'
                />
                {t('Create API key')}
              </Button>
            </div>
          </TabsContent>

          <TabsContent value='cc-switch' className='mt-3 grid gap-3'>
            <Alert>
              <AlertTitle>
                {t('Use official CC Switch downloads only')}
              </AlertTitle>
              <AlertDescription>
                {t(
                  'CC Switch is free and open source. An installer asking for payment, top-ups, or account credentials is not official.'
                )}
              </AlertDescription>
            </Alert>
            {ccSwitchInstall.command ? (
              <CodeSnippet
                label={t('Install CC Switch on {{platform}}', {
                  platform: PLATFORM_LABELS[platform],
                })}
                value={ccSwitchInstall.command}
              />
            ) : (
              <div className='bg-muted/40 rounded-lg border p-3 text-xs leading-5'>
                {t(
                  'Download {{artifact}} from GitHub Releases, open it, and finish the Windows installer.',
                  { artifact: ccSwitchInstall.artifact }
                )}
              </div>
            )}
            <Alert>
              <AlertTitle>{t('CC Switch one-click import')}</AlertTitle>
              <AlertDescription>
                {t(
                  'Create an API key first, then confirm the CC Switch import. The browser builds the link from the selected model and service root; the key never enters assistant chat.'
                )}
              </AlertDescription>
            </Alert>
            <ol className='grid gap-3' aria-label={t('CC Switch setup steps')}>
              <SetupStep
                number={1}
                title={t('Open the official CC Switch app')}
                description={t(
                  'Install and launch CC Switch from the official release or package.'
                )}
              />
              <SetupStep
                number={2}
                title={t('Create or select an API key')}
                description={t(
                  "Use Create API key below, or use the key's CC Switch action on the /keys page."
                )}
              />
              <SetupStep
                number={3}
                title={t('Build the import link')}
                description={t(
                  'Click Import to CC Switch, then review and confirm the import dialog.'
                )}
              />
              <SetupStep
                number={4}
                title={t('Enable and test Claude Code')}
                description={t(
                  'Run claude in a new terminal and send a short test message. If first-run login appears, enable Skip Claude Code first-run confirmation in CC Switch settings.'
                )}
              />
            </ol>
            <div className='flex flex-wrap gap-2'>
              <OfficialLink
                href={CC_SWITCH_RELEASES}
                label={t('Open official releases')}
              />
              <OfficialLink
                href={CC_SWITCH_INSTALL_DOCS}
                label={t('Installation manual')}
              />
              <OfficialLink
                href={CC_SWITCH_PROVIDER_DOCS}
                label={t('Provider manual')}
              />
              <Button
                size='sm'
                variant='outline'
                onClick={props.onCreateKey}
                disabled={!props.developerAccessGranted}
              >
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  data-icon='inline-start'
                  aria-hidden='true'
                />
                {t('Create API key')}
              </Button>
            </div>
          </TabsContent>

          <TabsContent value='claude-desktop' className='mt-3 grid gap-3'>
            {platform === 'linux' ? (
              <Alert>
                <AlertTitle>
                  {t(
                    'CC Switch Desktop provider setup is not available on Linux'
                  )}
                </AlertTitle>
                <AlertDescription>
                  {t(
                    'Claude Desktop for Linux is available in beta, but CC Switch currently writes third-party Desktop profiles only on Windows and macOS. Use Claude Code on Linux for this service.'
                  )}
                </AlertDescription>
              </Alert>
            ) : (
              <>
                <ol
                  className='grid gap-3'
                  aria-label={t('Claude Desktop setup steps')}
                >
                  <SetupStep
                    number={1}
                    title={t('Install Claude Desktop')}
                    description={t(
                      'Download the official app for {{platform}}, install it, and launch it once.',
                      { platform: PLATFORM_LABELS[platform] }
                    )}
                  />
                  <SetupStep
                    number={2}
                    title={t('Open Claude Desktop in CC Switch')}
                    description={t(
                      'In the CC Switch app switcher, select Claude Desktop. If it is hidden, enable it under Settings, General, Homepage Display.'
                    )}
                  />
                  <SetupStep
                    number={3}
                    title={t('Import the Claude Code provider')}
                    description={t(
                      'Choose Import existing providers from Claude Code, or add a custom provider with the endpoint and API key below.'
                    )}
                  />
                  <SetupStep
                    number={4}
                    title={t('Enable model mapping')}
                    description={t(
                      'Turn on Needs model mapping, map the Sonnet role to {{model}}, and enable Claude Desktop local routing.',
                      { model }
                    )}
                  />
                  <SetupStep
                    number={5}
                    title={t('Enable, then restart Desktop')}
                    description={t(
                      'Keep CC Switch running, enable the provider, fully quit Claude Desktop, and open it again.'
                    )}
                  />
                </ol>
                <div className='rounded-lg border px-3'>
                  <ConnectionValue
                    label={t('API endpoint root')}
                    value={props.rootUrl}
                  />
                  <ConnectionValue
                    label={t('API key')}
                    value='<YOUR_API_KEY>'
                  />
                  <ConnectionValue
                    label={t('Sonnet requested model')}
                    value={model}
                  />
                  <ConnectionValue
                    label={t('API format')}
                    value='Anthropic Messages'
                  />
                </div>
              </>
            )}
            <div className='flex flex-wrap gap-2'>
              <OfficialLink
                href={CLAUDE_DESKTOP_DOCS}
                label={t('Official Desktop guide')}
              />
              <OfficialLink
                href={CC_SWITCH_DESKTOP_DOCS}
                label={t('CC Switch Desktop manual')}
              />
              <Button
                size='sm'
                variant='outline'
                onClick={props.onCreateKey}
                disabled={!props.developerAccessGranted}
              >
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  data-icon='inline-start'
                  aria-hidden='true'
                />
                {t('Create API key')}
              </Button>
            </div>
          </TabsContent>

          <TabsContent value='chatgpt' className='mt-3 grid gap-3'>
            {!chatGPTDesktopAvailable ? (
              <Alert>
                <AlertTitle>
                  {t(
                    'The official ChatGPT desktop app is not available for Linux'
                  )}
                </AlertTitle>
                <AlertDescription>
                  {t(
                    'OpenAI currently provides ChatGPT desktop installers for Windows and macOS. On Linux, use chatgpt.com in your browser or choose an API-compatible client such as CC Switch.'
                  )}
                </AlertDescription>
              </Alert>
            ) : null}
            <div className='bg-muted/40 rounded-lg border p-3'>
              <div className='flex items-center gap-2 text-sm font-medium'>
                <HugeiconsIcon
                  icon={ComputerTerminal01Icon}
                  className='size-4'
                  strokeWidth={2}
                  aria-hidden='true'
                />
                {t('Official ChatGPT desktop uses OpenAI sign-in')}
              </div>
              <p className='text-muted-foreground mt-1 text-xs leading-5'>
                {t(
                  'The official ChatGPT desktop app does not accept this service Base URL and API key as a custom provider. Use CC Switch or an OpenAI-compatible client for this service.'
                )}
              </p>
            </div>
            <ol className='grid gap-3' aria-label={t('ChatGPT setup steps')}>
              <SetupStep
                number={1}
                title={
                  chatGPTDesktopAvailable
                    ? t('Install the official ChatGPT app')
                    : t('Use ChatGPT in your browser')
                }
                description={
                  chatGPTDesktopAvailable
                    ? t(
                        'Open the official download page, choose a supported desktop installer, install it, and sign in with your OpenAI account.'
                      )
                    : t(
                        'Open chatgpt.com in a supported browser and sign in with your OpenAI account. No Linux desktop installer is required.'
                      )
                }
              />
              <SetupStep
                number={2}
                title={t('Use a compatible client for this API')}
                description={t(
                  'To spend your balance on this service, use CC Switch, Claude Code, or another OpenAI-compatible client with the values below.'
                )}
              />
            </ol>
            <div className='rounded-lg border px-3'>
              <ConnectionValue
                label={t('OpenAI-compatible Base URL')}
                value={props.openAIBaseUrl}
              />
              <ConnectionValue label={t('Model ID')} value={model} />
              <ConnectionValue label={t('API key')} value='<YOUR_API_KEY>' />
            </div>
            <div className='flex flex-wrap gap-2'>
              <OfficialLink
                href={chatGPTDesktopAvailable ? CHATGPT_DOWNLOAD : CHATGPT_WEB}
                label={
                  chatGPTDesktopAvailable
                    ? t('Download official ChatGPT')
                    : t('Open ChatGPT in browser')
                }
              />
              <Button
                size='sm'
                variant='outline'
                onClick={() => setClientTab('cc-switch')}
              >
                {t('Use CC Switch instead')}
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  strokeWidth={2}
                  data-icon='inline-end'
                  aria-hidden='true'
                />
              </Button>
            </div>
          </TabsContent>

          <TabsContent value='codex' className='mt-3 grid gap-3'>
            <CodeSnippet
              label={t('Install command')}
              value={codexInstallCommand}
            />
            <CodeSnippet label={t('API key')} value={codexAPIKeyCommand} />
            <CodeSnippet
              label={t('Codex configuration: {{path}}', {
                path: codexConfigPath,
              })}
              value={codexConfig}
            />
            <ol className='grid gap-3' aria-label={t('Codex setup steps')}>
              <SetupStep
                number={1}
                title={t('Install Codex')}
                description={t(
                  'Install the official CLI, then verify it with codex --version.'
                )}
              />
              <SetupStep
                number={2}
                title={t('Add the LMM provider')}
                description={t(
                  'Save the provider block in the user-level config file. Codex uses the Responses API, so keep the /v1 Base URL.'
                )}
              />
              <SetupStep
                number={3}
                title={t('Set the API key in your shell')}
                description={t(
                  'Run the API-key command in the current terminal. The secret is not stored in config.toml.'
                )}
              />
              <SetupStep
                number={4}
                title={t('Verify the active provider')}
                description={t(
                  'Run codex in a project and use /status to verify the LMM provider and selected model.'
                )}
              />
            </ol>
            <div className='flex flex-wrap gap-2'>
              <OfficialLink
                href={CODEX_DOCS}
                label={t('Official Codex guide')}
              />
              <OfficialLink
                href={CODEX_CONFIG_DOCS}
                label={t('Configuration reference')}
              />
              <Button size='sm' variant='outline' onClick={props.onCreateKey}>
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  data-icon='inline-start'
                  aria-hidden='true'
                />
                {t('Create API key')}
              </Button>
            </div>
          </TabsContent>

          <TabsContent value='openai-compatible' className='mt-3 grid gap-3'>
            <Alert>
              <AlertTitle>
                {t('Codex, Cursor, Open WebUI, and more')}
              </AlertTitle>
              <AlertDescription>
                {t(
                  'These clients use an OpenAI-compatible provider. The field names may differ, but the three values below stay the same.'
                )}
              </AlertDescription>
            </Alert>
            <ol
              className='grid gap-3'
              aria-label={t('OpenAI-compatible client setup steps')}
            >
              <SetupStep
                number={1}
                title={t('Install the official client')}
                description={t(
                  'Use the official download or installation guide for Codex, Cursor, Open WebUI, or another compatible client.'
                )}
              />
              <SetupStep
                number={2}
                title={t('Choose a custom provider')}
                description={t(
                  'Open the client provider, model, or API settings and select its custom OpenAI-compatible option.'
                )}
              />
              <SetupStep
                number={3}
                title={t('Paste the three connection values')}
                description={t(
                  'Paste the Base URL, exact Model ID, and API key shown below. Keep the API key private.'
                )}
              />
              <SetupStep
                number={4}
                title={t('Run a small test request')}
                description={t(
                  'Verify the connection with a short test before enabling automation, agents, or long-running tasks.'
                )}
              />
            </ol>
            <div className='rounded-lg border px-3'>
              <ConnectionValue
                label={t('OpenAI-compatible Base URL')}
                value={props.openAIBaseUrl}
              />
              <ConnectionValue label={t('Model ID')} value={model} />
              <ConnectionValue label={t('API key')} value='<YOUR_API_KEY>' />
            </div>
            <CodeSnippet
              label={t('OpenAI-compatible connection JSON')}
              value={openAICompatibleConfig}
            />
            <p className='text-muted-foreground text-xs leading-5'>
              {t(
                'If a client needs a product-specific configuration, ask the AI assistant for that client and platform. It will use the current Base URL and Model ID instead of guessing.'
              )}
            </p>
            <div className='flex flex-wrap gap-2'>
              <OfficialLink href={CODEX_DOCS} label='Codex' />
              <OfficialLink href={CURSOR_DOWNLOADS} label='Cursor' />
              <OfficialLink href={OPEN_WEBUI_GUIDE} label='Open WebUI' />
              <Button
                size='sm'
                variant='outline'
                onClick={props.onCreateKey}
                disabled={!props.developerAccessGranted}
              >
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  data-icon='inline-start'
                  aria-hidden='true'
                />
                {t('Create API key')}
              </Button>
            </div>
          </TabsContent>
        </Tabs>
      </CardContent>
    </Card>
  )
}
