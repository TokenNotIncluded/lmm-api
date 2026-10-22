/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
type Language = 'curl' | 'python' | 'typescript' | 'javascript'
type Context = {
  baseUrl: string
  apiKeyEnv: string
  modelName: string
  endpointType: 'deepseek-search' | 'deepseek-fetch'
  endpointPath: string
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`
}

export function buildNativeToolSample(lang: Language, ctx: Context): string {
  const url = `${ctx.baseUrl}${ctx.endpointPath}`
  const keyEnv = /^[A-Z_][A-Z_0-9]*$/.test(ctx.apiKeyEnv)
    ? ctx.apiKeyEnv
    : 'LMM_API_KEY'
  const body = JSON.stringify(
    {
      model: ctx.modelName,
      ...(ctx.endpointType === 'deepseek-fetch'
        ? { url: 'https://example.com/' }
        : { query: 'quantum entanglement' }),
    },
    null,
    2
  )
  if (lang === 'curl') {
    return [
      `curl ${shellQuote(url)} \\`,
      `  -H "Authorization: Bearer $${keyEnv}" \\`,
      '  -H "Content-Type: application/json" \\',
      `  -d ${shellQuote(body)}`,
    ].join('\n')
  }
  if (lang === 'python') {
    return [
      'import json',
      'import os',
      'from urllib.request import Request, urlopen',
      '',
      `payload = ${body}`,
      'request = Request(',
      `    ${JSON.stringify(url)},`,
      '    data=json.dumps(payload).encode("utf-8"),',
      '    headers={',
      `        "Authorization": f"Bearer {os.environ['${keyEnv}']}",`,
      '        "Content-Type": "application/json",',
      '    },',
      '    method="POST",',
      ')',
      '',
      'with urlopen(request) as response:',
      '    print(json.load(response))',
    ].join('\n')
  }
  return [
    `const response = await fetch(${JSON.stringify(url)}, {`,
    "  method: 'POST',",
    '  headers: {',
    `    Authorization: \`Bearer \${process.env.${keyEnv}}\`,`,
    "    'Content-Type': 'application/json',",
    '  },',
    `  body: JSON.stringify(${body}),`,
    '})',
    '',
    'if (!response.ok) throw new Error(`Request failed: ${response.status}`)',
    'console.log(await response.json())',
  ].join('\n')
}
