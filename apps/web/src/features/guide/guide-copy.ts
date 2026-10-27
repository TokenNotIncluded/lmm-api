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
import { useTranslation } from 'react-i18next'

export function useGuideCopy() {
  const { t } = useTranslation()

  return {
    eyebrow: t('Getting started'),
    title: t('Connect once. Start building.'),
    intro: t(
      'Pick your device and app, then follow the steps to your first request.'
    ),
    start: t('Choose my client'),
    assistant: t('Guide me with AI'),
    manualIntro: t(
      'Pick your device and app, then follow the steps. Support is here if you get stuck.'
    ),
    sectionsTitle: t('On this page'),
    sections: [
      t('Choose a client'),
      t('Send your first message'),
      t('Common errors'),
      t('Account and support'),
    ],
    contactSupport: t('Contact support'),
    requestManualAccess: t('Contact support for access'),
    supportTitle: t('Need help from a person?'),
    supportBody: t(
      'Email support with your account, device and issue. Hide your API key.'
    ),
    supportTicket: t('Open a support ticket'),
    manualTroubleBody: t(
      'Open the matching error code below, or contact support if still stuck.'
    ),
    guestAssistant: t('Sign in for AI guidance'),
    stages: [
      t('Choose an app'),
      t('Download and install'),
      t('Request access and create a key'),
      t('Configure your client'),
      t('Send your first message'),
    ],
    setupTitle: t('Start with the device you use'),
    setupBody: t(
      'Pick your device and app for official downloads and setup steps.'
    ),
    accountTitle: t('Prepare your account'),
    accountBody: t(
      'Once access is approved, create a key to connect LMM to your app.'
    ),
    accountSteps: [
      t('Get API access'),
      t('Create an API key'),
      t('Complete your first request'),
    ],
    complete: t('Complete'),
    pending: t('Pending'),
    signIn: t('Sign in to continue setup'),
    requestAccess: t('Ask the assistant for access'),
    createKey: t('Create a key and import'),
    requestAccessQuestion: t(
      'I am reading the getting-started guide and want to use the LMM API. Explain what information is needed for access and guide me through the application.'
    ),
    createKeyQuestion: t(
      'I have read the getting-started guide. Help me create an API key for my first connection, explain permissions, limits and routing groups, then guide me through importing it into my chosen client.'
    ),
    setupQuestion: t(
      'I am new to LMM. First ask which device and app I use and what I want to do, then guide me through the official download, installation, access request, key creation, base URL and model configuration, and my first message. Explain where to click and how to verify each step.'
    ),
    securityTitle: t('Enter keys only in trusted clients'),
    securityBody: t(
      'Leaked a key? Revoke it in API Keys. Share the request ID, never the key.'
    ),
    modelsError: t(
      'Models did not load. Keep reading the steps, then retry before configuring.'
    ),
    retry: t('Reload models'),
    firstTitle: t('Send a message to check your connection'),
    firstBody: t(
      'Save your settings, start a new conversation, and send the message below.'
    ),
    firstMessage: t(
      'Hello! Tell me in one sentence what you can help me with.'
    ),
    usage: t('Open usage logs'),
    modelList: t('Explore models and pricing'),
    troubleTitle: t('Stuck on a step? Start here.'),
    troubleBody: t(
      'Open the matching error code, or hand the details to the assistant.'
    ),
    troubles: [
      {
        code: '401',
        title: t('API key authentication failed'),
        body: t(
          'Copy your API key again, remove surrounding spaces, and check it has not expired or been revoked. Enter it in the app’s API Key field. If it still fails, share the error and request ID with the key hidden.'
        ),
        question: t(
          'I received HTTP 401 while setting up my client. Ask for my device, client, error message and request ID without asking for my API key, then help me check key entry, expiration and permissions step by step.'
        ),
      },
      {
        code: '404',
        title: t('Check the endpoint and model'),
        body: t(
          'Follow the address instructions for the selected client and check that /v1 is not duplicated. Use an exact model ID from your account’s available model list, then retry once.'
        ),
        question: t(
          'I received HTTP 404 while setting up my client. Ask for the client, Base URL, model ID and error message, then help me check endpoint paths and model availability. Do not ask for my API key.'
        ),
      },
      {
        code: '429',
        title: t('A request limit was reached'),
        body: t(
          'Pause repeated retries and read the error details. For rate limits, wait and reduce concurrency. For insufficient quota, check key limits, balance and your plan. The error details help distinguish the cause.'
        ),
        question: t(
          'I received HTTP 429 on my first connection. Use the error details to identify rate limits, quota or upstream throttling and give specific next steps. Remind me to hide the key before sharing the error.'
        ),
      },
    ],
    askAboutError: t('Troubleshoot with AI'),
    guestError: t('Sign in to troubleshoot with AI'),
    developerTitle: t('For developers: test with curl'),
    developerBody: t(
      'Run in a local Bash-compatible terminal. Replace the example key and model ID; the request is billed at the selected model’s rate.'
    ),
    copy: t('Copy'),
    copied: t('Copied'),
    copyFailed: t('Copy failed. Select the content and copy it manually.'),
    workspaceLabel: t('Connection guide'),
    quickStart: t('Quick start'),
    reference: t('API reference'),
    endpoint: t('API endpoint'),
    endpointHint: t('Use the address required by your selected client.'),
    browseModels: t('Browse models'),
    setupLabel: t('Client setup'),
    messageLabel: t('First request'),
    troubleshootLabel: t('Troubleshooting'),
    supportLabel: t('Account & support'),
    checklistLabel: t('Before you connect'),
    requestLabel: t('Request example'),
    chooseClientHint: t('Choose your app. Follow your steps.'),
  }
}

export type GuideCopy = ReturnType<typeof useGuideCopy>
