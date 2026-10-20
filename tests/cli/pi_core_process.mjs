// Isolated process runner: actual Pi provider and Pi SDK, actual Chromium and Core HTTP.
// Only the supplier and clearly labelled login form are synthetic; no API responses are mocked.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFileSync,writeFileSync,existsSync,mkdirSync} from 'node:fs';
import {join} from 'node:path';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {registerHooks} from 'node:module';
import {createHash} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
const config=JSON.parse(readFileSync(process.argv[2],'utf8'));
const root=config.output;
const record=(name,value)=>writeFileSync(join(root,name+'.json'),JSON.stringify(value,null,2)+'\n',{mode:0o600,flag:'wx'});
const hash=(value)=>createHash('sha256').update(value).digest('hex');
const modules=new Map();
registerHooks({resolve(specifier,context,next){
 const value=specifier.startsWith('@earendil-works/')
  ?next(specifier,{...context,parentURL:pathToFileURL(join(config.sdk,'package.json')).href})
  :next(specifier,context);
 if(value.url.startsWith('file:')&&/\.(?:js|mjs|cjs|ts|json)$/.test(new URL(value.url).pathname)){
  const before=hash(readFileSync(fileURLToPath(value.url)));
  if(modules.has(value.url))assert.equal(modules.get(value.url),before,'loaded module changed');
  modules.set(value.url,before);
 }
 return value;
}});
const {LmmIntegration}=await import(pathToFileURL(join(config.provider,'src/provider.ts')));
const {normalizeContext}=await import('@earendil-works/pi-ai/utils/transcript');
const {chromium}=await import(pathToFileURL(join(config.playwright,'index.mjs')));
let hits=0;
let stage='transport-start';
const supplier=[];
const browserNetwork=[];
let browser,context,integration;
const abort=new AbortController();
const servers=[];
const close=async()=>{
 abort.abort();integration?.dispose();
 if(context) await context.close().catch(()=>{});
 if(browser) await browser.close().catch(()=>{});
 for(const server of servers) await new Promise(resolve=>{server.close(resolve);server.closeAllConnections();});
};
process.once('SIGTERM',()=>{void close().finally(()=>process.exit(3));});
function server(handler,port){
 const value=createServer((req,res)=>{void handler(req,res).catch(()=>{res.writeHead(500);res.end('isolated fixture failed');});});
 servers.push(value);
 return new Promise((resolve,reject)=>{value.once('error',reject);value.listen(port,'127.0.0.1',()=>resolve(value));});
}
async function body(request){let raw='';for await(const part of request){raw+=part;if(Buffer.byteLength(raw)>65536)throw Error('bounded request');}return raw;}
const escape=(value)=>value.replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;').replaceAll('>','&gt;');
const noProxyFetch=(url,options)=>fetch(url,{...options,redirect:'error'});
try {
 assert.equal(JSON.parse(readFileSync(join(config.sdk,'package.json'))).version,'1.2.0');
 assert.match(config.issuer,/^http:\/\/127\.0\.0\.1:\d+$/);
 await server(async(req,res)=>{
  assert.equal(req.url,'/upstream/v1/chat/completions');
  const value=JSON.parse(await body(req));
  assert.equal(req.headers.authorization,`Bearer ${config.supplier_secret}`);
  assert.equal(value.model,'gpt-4o-mini');assert.equal(value.stream,true);
  hits++;
  supplier.push({sequence:hits,body_sha256:hash(JSON.stringify(value)),synthetic_supplier_credential:true,oauth_token_not_forwarded:!req.headers.authorization.includes('lmm_at_')});
  res.writeHead(200,{'content-type':'text/event-stream','cache-control':'no-store'});
  for(const frame of [
   {id:'isolated-'+hits,object:'chat.completion.chunk',created:0,model:'gpt-4o-mini',choices:[{index:0,delta:{role:'assistant',content:'isolated hello'},finish_reason:null}]},
   {id:'isolated-'+hits,object:'chat.completion.chunk',created:0,model:'gpt-4o-mini',choices:[{index:0,delta:{},finish_reason:'stop'}]},
   {id:'isolated-'+hits,object:'chat.completion.chunk',created:0,model:'gpt-4o-mini',choices:[],usage:{prompt_tokens:10,completion_tokens:20,total_tokens:30,prompt_tokens_details:{cached_tokens:2}}},
  ]) res.write('data: '+JSON.stringify(frame)+'\n\n');
  res.end('data: [DONE]\n\n');
 },config.upstream_port);
 await server(async(req,res)=>{
  if(req.method==='GET'&&req.url==='/sign-in'){
   res.writeHead(200,{'content-type':'text/html; charset=utf-8','cache-control':'no-store','content-security-policy':"default-src 'none'; form-action 'self'; frame-ancestors 'none'"});
   res.end(`<title>Isolated synthetic login harness</title><h1>Synthetic login harness</h1><p>This is a fixture login form, not the product Web sign-in UI. It submits to the real isolated Core login API.</p><form method=post action=/login><label>Fixture username<input name=username value="${escape(config.username)}" readonly></label><label>Fixture password<input type=password name=password value="${escape(config.password)}" readonly></label><button>Sign in to isolated Core</button></form>`);return;
  }
  if(req.method==='POST'&&req.url==='/login'){
   const form=new URLSearchParams(await body(req));
   assert.equal(form.get('username'),config.username);assert.equal(form.get('password'),config.password);
   const response=await noProxyFetch(config.issuer+'/api/user/login',{method:'POST',headers:{'content-type':'application/json',origin:config.issuer},body:JSON.stringify({username:form.get('username'),password:form.get('password')})});
   assert.equal(response.status,200);assert.equal((await response.json()).success,true);
   const cookies=response.headers.getSetCookie();assert.ok(cookies.length>0);
   res.writeHead(200,{'content-type':'text/html','cache-control':'no-store','set-cookie':cookies});res.end('<h1>Signed in to isolated Core</h1><p>Return to the real authorization page and explicitly continue.</p>');return;
  }
  res.writeHead(404);res.end();
 },config.harness_port);
 record('transport-ready',{upstream_port:config.upstream_port,harness_port:config.harness_port,issuer:config.issuer});
 for(let attempt=0;!existsSync(join(root,'begin-login.json'));attempt++){assert.ok(attempt<600,'Core start/seed gate timed out');await delay(100);}
 stage='discovery';
 const metadata=await (await noProxyFetch(config.issuer+'/.well-known/oauth-authorization-server')).json();
 assert.equal(metadata.lmm_deployment_contract,'isolated-loopback-test-v1');assert.equal(metadata.lmm_isolated_loopback_test,true);
 stage='provider-login';
 integration=new LmmIntegration({issuer:config.issuer,allowLoopbackHttpForTests:true,loginTimeoutMs:120000,refreshJournalDirectory:join(config.secrets,'refresh-journal')});
 let authorize;
 const authorization=new Promise(resolve=>{authorize=resolve;});
 const login=integration.provider.auth.oauth.login({signal:abort.signal,notify(event){if(event.type==='auth_url')authorize(event.url);}});
 // Attach immediately so a browser-launch failure cannot leave a rejected login unobserved.
 const loginResult=login.then(value=>({ok:true,value}),()=>({ok:false}));
 const authURL=new URL(await authorization);
 assert.equal(authURL.origin,config.issuer);assert.equal(authURL.searchParams.get('client_id'),'lmm-pi');assert.equal(authURL.searchParams.get('code_challenge_method'),'S256');
 const callback=new URL(authURL.searchParams.get('redirect_uri'));assert.equal(callback.hostname,'127.0.0.1');assert.equal(callback.pathname,'/oauth/lmm/callback');
 stage='browser-launch';
 browser=await chromium.launch({executablePath:config.chromium,headless:true,args:['--disable-background-networking','--disable-component-update','--no-first-run','--password-store=basic','--use-mock-keychain','--disable-gpu']});
 context=await browser.newContext({acceptDownloads:false,viewport:{width:1100,height:800}});
 const allowed=new Set([config.issuer,`http://127.0.0.1:${config.harness_port}`,callback.origin]);
 await context.route('**/*',route=>allowed.has(new URL(route.request().url()).origin)?route.continue():route.abort());
 context.on('response',response=>{const url=new URL(response.url());browserNetwork.push({origin:url.origin,path:url.pathname,status:response.status()});});
 stage='browser-authorize';
 const authPage=await context.newPage();await authPage.goto(authURL.href);
 stage='browser-consent';
 await authPage.getByRole('button',{name:'Continue after sign in'}).waitFor();
 record('browser-initial-page',{url_origin:authURL.origin,text:await authPage.locator('body').innerText(),real_core_product_html:true});
 stage='browser-login-harness';
 const loginPage=await context.newPage();await loginPage.goto(`http://127.0.0.1:${config.harness_port}/sign-in`);
 await loginPage.getByRole('button',{name:'Sign in to isolated Core'}).click();await loginPage.getByRole('heading',{name:'Signed in to isolated Core'}).waitFor();
 stage='browser-consent';
 await authPage.getByRole('button',{name:'Continue after sign in'}).click();
 await authPage.locator('input[name=budget_credits]').fill('200');
 const consentText=await authPage.locator('body').innerText();assert.ok(consentText.includes('Allow model calls and usage'));assert.ok(consentText.includes(config.username));
 await authPage.screenshot({path:join(root,'real-core-consent.png')});
 record('browser-consent-page',{origin:config.issuer,text:consentText,budget_credits:200,real_core_product_html:true});
 await authPage.getByRole('button',{name:'Allow scoped model access'}).click();
 await authPage.waitForURL(url=>url.pathname==='/oauth/lmm/callback');
 stage='provider-token';
 const issued=await loginResult;assert.equal(issued.ok,true);const credential=issued.value;
 assert.equal(credential.lmm_issuer,config.issuer);assert.ok(credential.scope.split(' ').includes('models:invoke'));
 assert.ok(!credential.scope.split(' ').includes('remote:control'));assert.ok(!credential.scope.split(' ').includes('mcp:bounties'));
 stage='provider-catalog';
 const publication={credential,allowNetwork:true,signal:abort.signal,publish:async value=>{value.update?.();return true;}};
 await integration.provider.refreshModels(publication);
 const models=integration.provider.getModels();assert.equal(models.length,1);
 let auth=await integration.provider.auth.oauth.toAuth(credential);
 const cases=[];
 async function modelCall(label){
  const events=[];
  for await(const event of integration.provider.streamSimple(models[0],normalizeContext({messages:[{role:'user',content:'isolated '+label,timestamp:Date.now()}]}),{headers:auth.headers,maxTokens:20,signal:abort.signal}))events.push(event);
  return events;
 }
 stage='provider-model-stream';
 for(let index=0;index<2;index++){
  const events=await modelCall('call-'+index);const done=events.find(value=>value.type==='done');assert.ok(done,'actual provider stream must complete');
  assert.equal(done.message.stopReason,'stop');assert.ok(done.message.content.some(value=>value.type==='text'&&value.text==='isolated hello'));
  cases.push({label:'stream-'+index,events:events.map(value=>value.type),completed:true});
 }
 assert.equal(hits,2);
 stage='provider-refresh';
 const refreshed=await integration.oauth.refresh(credential,abort.signal);assert.notEqual(refreshed.access,credential.access);assert.notEqual(refreshed.refresh,credential.refresh);assert.equal(refreshed.scope,credential.scope);
 auth=await integration.provider.auth.oauth.toAuth(refreshed);
 stage='provider-budget-denied-after-refresh';
 const denied=await modelCall('budget-denied-after-refresh');assert.ok(denied.some(value=>value.type==='error'));assert.equal(hits,2);
 const balance=await integration.http.bearer('/api/oauth2/balance',refreshed.access,abort.signal);assert.equal(balance.quota,902);
 stage='provider-revoke';
 await integration.oauth.revoke(refreshed.access,abort.signal);
 const rejected=await fetch(config.issuer+'/api/oauth2/catalog',{headers:{authorization:`Bearer ${refreshed.access}`},redirect:'error'});assert.equal(rejected.status,401);
 await assert.rejects(integration.oauth.refresh(refreshed,abort.signal));
 for(const [url,before]of modules)assert.equal(hash(readFileSync(fileURLToPath(url))),before,'module drifted during actual journey');
 record('loaded-module-binding',Object.fromEntries(modules));
 record('provider-result',{status:'passed',actual_pi_provider:true,actual_pi_sdk:true,real_chromium:true,browser_version:browser.version(),browser_context_fresh:true,login_ui:'synthetic-harness-to-real-Core-login',authorization_ui:'real-Core-product-HTML',callback_validated_by_actual_provider:true,pkce:'S256',calls:cases,budget_credits:200,budget_denial_events:denied.map(value=>value.type),supplier_hits:hits,supplier,refresh_rotated:true,budget_carried_across_refresh:true,revocation_denied_catalog:true,post_revoke_refresh_denied:true,balance_after_credits:balance.quota,browser_network:browserNetwork,production_contact:false});
} catch(error){record('provider-failure',{status:'failed',stage,error_type:error?.constructor?.name??'Unknown',supplier_hits:hits,production_contact:false});process.exitCode=3;}
finally{await close();}
