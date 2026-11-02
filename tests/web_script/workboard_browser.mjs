// Manual real-browser check against a fresh loopback server and scratch DB.
// KKT_PLAYWRIGHT_MODULE may name an independently installed Playwright module.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
const modulePath = process.env.KKT_PLAYWRIGHT_MODULE;
const { chromium } = await import(modulePath ? pathToFileURL(resolve(modulePath)).href : "playwright");
const output = resolve("target/k1-validation");
await mkdir(output, {recursive:true});
const scratch = await mkdtemp(output + "/browser-smoke-");
await writeFile(scratch + "/config.toml", "");
const child = spawn(resolve("target/debug/kkt"), ["serve", "--config", scratch + "/config.toml", "--ignore-setup", "--db", scratch + "/server.db", "--bind", "127.0.0.1:0", "--no-access-key"], {stdio:["ignore","pipe","pipe"]});
let log = "";
child.stderr.on("data", data => { log += data; });
let browser;
try {
 const base = await new Promise((resolve,reject) => {
  const timer = setTimeout(() => reject(new Error("Scratch server startup timeout: " + log)),20000);
  child.stdout.on("data", data => {
   log += data;
   const match = log.match(/bound to (127\.0\.0\.1:\d+)/);
   if(match) {clearTimeout(timer); resolve("http://"+match[1]);}
  });
  child.once("exit", code => {clearTimeout(timer); reject(new Error("Scratch server exited " + code + ": " + log));});
 });
 assert.equal(new URL(base).hostname,"127.0.0.1");
 let session, sequence = 0;
 async function rpc(method, params, notification = false) {
  const headers = {"Content-Type":"application/json", "Accept":"application/json, text/event-stream"};
  if(session) headers["Mcp-Session-Id"] = session;
  const body = {jsonrpc:"2.0",method,params};
  if(!notification) body.id = ++sequence;
  const response = await fetch(base+"/mcp",{method:"POST",headers,body:JSON.stringify(body)});
  assert.ok(response.ok,"Local MCP request failed: "+response.status);
  session = response.headers.get("mcp-session-id") || session;
  const text = await response.text();
  if(notification) return;
  const message = text.startsWith("{") ? JSON.parse(text) : text.split("\n").filter(line=>line.startsWith("data:")).map(line=>JSON.parse(line.slice(5))).find(value=>value.id===body.id);
  assert.ok(message && !message.error,JSON.stringify(message?.error));
  return message.result;
 }
 await rpc("initialize",{protocolVersion:"2025-03-26",capabilities:{},clientInfo:{name:"workboard-browser-smoke",version:"1"}});
 await rpc("notifications/initialized",{},true);
 async function call(name,args) {
  const result = await rpc("tools/call",{name,arguments:{...args,format:"json"}});
  assert.ok(!result.isError,result.content?.map(c=>c.text).join("\n"));
  return JSON.parse(result.content.find(c=>c.type==="text").text);
 }
 const agent = await call("register",{handle:"ui-smoke-agent"});
 const channel = await call("create_channel",{token:agent.token,name:"Browser validation",open:true});
 const item = agent.handle + "/latency";
 let request = 0;
 async function send(report) {
  return await call("send",{token:agent.token,channel_id:channel.id,body:"kkt-workboard/1\n"+JSON.stringify({request_id:"browser-"+(++request),item_id:item,...report})});
 }
 let created = await send({op:"create",kind:"task",payload:{title:"Initial browser report",state:"in_progress",owner:agent.handle}});
 let revision = created.message_id;
 assert.ok(Number.isInteger(revision));
 browser = await chromium.launch({headless:true,args:["--no-sandbox"]});
 const page = await browser.newPage({viewport:{width:1440,height:1100}});
 await page.goto(base+"/channels/"+channel.id+"/work");
 await page.waitForSelector("[data-work-item]");
 await page.waitForFunction(()=>["live","up"].includes(document.getElementById("live").dataset.state));
 const samples = [];
 for(let n=1;n<=5;n++) {
  const title = "Updated browser report " + n;
  const started = performance.now();
  const sent = await send({op:"update",base_message_id:revision,payload:{title,state:"in_progress",owner:agent.handle}});
  revision = sent.message_id;
  await page.waitForFunction(title=>document.querySelector(".work-title")?.textContent===title,title);
  await page.evaluate(()=>new Promise(requestAnimationFrame));
  samples.push(performance.now()-started);
 }
 const summary = page.locator(".work-item summary").first();
 await summary.focus();
 await summary.evaluate(node=>{node.parentElement.open=true;window.workFocusedSummary=node;});
 await send({op:"verify",item_revision:revision,success:true,evidence_message_ids:[revision]});
 await page.waitForFunction(()=>document.querySelector(".work-freshness")?.textContent.includes("Verified at"));
 assert.ok(await summary.evaluate(node=>node===window.workFocusedSummary && node===document.activeElement),"Focused expandable row must survive verification refresh");
 await page.screenshot({path:output+"/browser-live-report.png",fullPage:true});
 const extras = await Promise.all(Array.from({length:16},(_,n)=>send({item_id:agent.handle+"/scroll-"+n,op:"create",kind:"task",payload:{title:"Scroll task "+n,state:"queued",owner:agent.handle}})));
 assert.equal(extras.length,16);
 await page.waitForFunction(()=>document.querySelectorAll(".work-item").length===17);
 const anchor = page.locator('[data-work-item="'+agent.handle+'/scroll-15"] summary');
 const anchorTop = await page.evaluate(id=>{
  const node=document.querySelector('[data-work-item="'+id+'"] summary');
  node.scrollIntoView({block:"center"});node.focus({preventScroll:true});
  if(document.activeElement!==node)throw new Error("Could not focus connected row");
  window.workScrollAnchor=node;return node.getBoundingClientRect().top;
 },agent.handle+"/scroll-15");
 await send({item_id:agent.handle+"/new-request",op:"create",kind:"decision",payload:{title:"New request above existing work",question:"Which scope should we use?",response_mode:"text",affected_task_ids:[]}});
 await page.waitForFunction(()=>document.querySelector(".work-inbox").textContent.includes("New request above existing work"));
 await page.evaluate(()=>new Promise(requestAnimationFrame));
 const anchorAfter = await anchor.evaluate(node=>({top:node.getBoundingClientRect().top,focused:node===document.activeElement,same:node===window.workScrollAnchor}));
 const focusDiagnostic = await page.evaluate(()=>({oldConnected:window.workScrollAnchor.isConnected,active:document.activeElement.outerHTML.slice(0,180)}));
 assert.ok(anchorAfter.focused && anchorAfter.same,"Focus identity survives a live burst and inserted global request: "+JSON.stringify({...anchorAfter,...focusDiagnostic}));
 assert.ok(Math.abs(anchorAfter.top-anchorTop)<2,"Scroll anchor moved by "+(anchorAfter.top-anchorTop)+" px");
 await page.setViewportSize({width:390,height:844});
 assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"Mobile view must not scroll sideways");
 const evidence = {samples_ms:samples.map(n=>Number(n.toFixed(2))),max_ms:Number(Math.max(...samples).toFixed(2)),target_ms:2000,iterations:5,focus_preserved:true,scroll_anchor_preserved:true,mobile_no_horizontal_overflow:true,scope:"fresh loopback server, scratch DB, genuine MCP send -> SSE -> snapshot -> Chromium frame; local simulated reports only"};
 await writeFile(output+"/browser-latency.json",JSON.stringify(evidence,null,2)+"\n");
 console.log(JSON.stringify(evidence));
} finally {
 if(browser) await browser.close();
 if(child.exitCode===null) {
  const done=once(child,"exit");child.kill("SIGTERM");
  await Promise.race([done,new Promise(resolve=>setTimeout(()=>{if(child.exitCode===null)child.kill("SIGKILL");resolve();},5000))]);
 }
 await writeFile(scratch+"/server.log",log);
}
