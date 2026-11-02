// Real rendered preview script, controlled fetch/events/time.
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
const base=process.argv[2];
if (!base) throw new Error("usage: node workboard.mjs <preview URL>");
const home=await (await fetch(base)).text();
const channel=[...home.matchAll(/href="\/channels\/(ch_[a-z0-9]+)"/g)][0]?.[1];
assert.ok(channel,"preview channel");
const html=await (await fetch(base+"/channels/"+channel+"/work")).text();
assert.ok(html.includes('id="workboard"'),"workboard page rendered");
const content=(revision,label="Task")=>'<section data-channel-id="'+channel+'" data-high-watermark="'+revision+'" data-evaluated-at="'+revision+'"><article data-work-item="agent-a/task" data-revision="'+revision+'"><a href="#task">'+label+'</a><details><summary>Details</summary>Evidence</details></article><article data-work-item="agent-a/done" data-group="History">Completed</article></section>';
assert.ok(html.includes("work-history"),"history filter rendered");
const settle=async()=>{for(let i=0;i<8;i++)await new Promise(r=>setImmediate(r));};
function harness() {
  const listeners={},timers=[],sources=[]; let requests=[],next=content(1),calls=0,hold=false;
  const dom=new JSDOM(html.replace(/data-high-watermark="[0-9]+"/g,'data-high-watermark="0"').replace(/data-evaluated-at="[0-9]+"/g,'data-evaluated-at="0"'),{url:base+"/channels/"+channel+"/work",runScripts:"dangerously",pretendToBeVisual:true,beforeParse(w){
    w.scrollTo=()=>{};
    w.setInterval=(fn,ms)=>{timers.push({fn,ms});return timers.length;};
    w.clearInterval=id=>{timers[id-1].cleared=true;};
    w.fetch=async()=>{calls++;if(hold)return await new Promise(resolve=>requests.push(resolve));return{ok:true,redirected:false,text:async()=>next};};
    w.EventSource=class {
      constructor(){this.readyState=1;sources.push(this);this.closed=false;}
      addEventListener(name,fn){(listeners[name]??=[]).push(fn);}
      close(){this.closed=true;}
    };
  }});
  return {
    dom,sources,root:()=>dom.window.document.getElementById("workboard"),calls:()=>calls,timers,
    event:(name,data={})=>{for(const fn of listeners[name]??[])fn({data:JSON.stringify(data)});},
    next:(n,label)=>{next=content(n,label);},
    nextHTML:html=>{next=html;},
    hold:()=>{hold=true;}, release:(n)=>{hold=false;const r=requests.shift();assert.ok(r,"pending fetch");r({ok:true,redirected:false,text:async()=>content(n)});},
    close:()=>dom.window.close(),
  };
}
{
 const h=harness();await settle();
 assert.ok(h.calls()>=1,"initial snapshot fetch");
 const history=h.dom.window.document.getElementById("work-history");
 assert.equal(h.root().querySelector('[data-group="History"]').hidden,true,"history hidden by default");
 history.checked=true;history.dispatchEvent(new h.dom.window.Event("change"));
 assert.equal(h.root().querySelector('[data-group="History"]').hidden,false,"history can be shown");

 assert.ok(h.timers.some(t=>t.ms===30000),"30-second reconciliation");
 h.next(2);h.event("message",{channel_id:channel,id:2});await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"2","live invalidation");
 h.next(1);h.event("open");await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"2","old response cannot replace new");
 h.next(3);h.event("lagged",1);await settle();assert.equal(h.root().querySelector("section").dataset.highWatermark,"3","lagged recovers");
 h.next(4);h.timers.find(t=>t.ms===30000).fn();await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"4","missed event periodic recovery");
 const anchor=h.root().querySelector("a");anchor.focus();
 h.next(5);h.event("message",{channel_id:channel,id:5});await settle();
 assert.equal(h.dom.window.document.activeElement,anchor,"focus node preserved");
 h.close();
}
{
 const h=harness();await settle();h.hold();h.event("open");await settle();
 h.event("message",{channel_id:channel,id:8});h.next(8);h.release(2);await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"8","event during fetch causes follow-up");
 h.close();
}


{
 const h=harness();await settle();
 h.event("error");h.next(2);h.timers.find(t=>t.ms===30000).fn();await settle();
 assert.equal(h.dom.window.document.getElementById("live").dataset.state,"down","poll success cannot claim SSE reconnected");
 h.close();
}

{
 const h=harness();await settle();
 const details=h.root().querySelector("details");details.open=true;
 h.next(1,"Expired; last check failed");h.event("open");await settle();
 assert.equal(h.root().querySelector("a").textContent,"Expired; last check failed","open details cannot suppress same-revision semantic changes");
 assert.equal(h.root().querySelector("details").open,true,"details stay open");
 h.close();
}



{
 const h=harness();await settle();
 const transition=(type,persisted)=>h.dom.window.dispatchEvent(new h.dom.window.PageTransitionEvent(type,{persisted}));
 transition("pagehide",true);
 assert.ok(h.sources.every(s=>s.closed),"cached navigation closes event streams");
 const before=h.calls();
 h.next(2);transition("pageshow",true);await settle();
 assert.ok(h.calls()>before,"bfcache restore resumes snapshot refresh");
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"2");
 assert.equal(h.sources.filter(s=>!s.closed).length,1,"restore creates one event stream");
 h.next(3);h.timers.filter(t=>!t.cleared).at(-1).fn();await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"3","restored reconciliation runs");
 transition("pagehide",false);
 assert.ok(h.sources.every(s=>s.closed),"final navigation closes every stream");
 assert.ok(h.timers.every(t=>t.cleared),"final navigation clears every timer");
 const stopped=h.calls();h.dom.window.dispatchEvent(new h.dom.window.Event("focus"));await settle();
 assert.equal(h.calls(),stopped,"final navigation prevents new fetches");
 h.close();
}

{
 const h=harness();await settle();
 h.root().querySelector("a").focus();
 h.nextHTML('<section data-channel-id="'+channel+'" data-high-watermark="0" data-evaluated-at="2" data-available="false"><p>Hidden channel; select hidden=1 to inspect history</p></section>');
 h.event("channel_hidden",{channel_id:channel});await settle();
 assert.equal(h.root().querySelectorAll("[data-work-item]").length,0,"hidden placeholder removes old focused reports");
 assert.ok(h.root().textContent.includes("hidden=1"),"hidden selection explains explicit opt-in");
 h.close();
}
{
 const h=harness();await settle();h.hold();h.event("open");await settle();
 h.dom.window.dispatchEvent(new h.dom.window.PageTransitionEvent("pagehide",{persisted:true}));
 h.next(3);
 h.dom.window.dispatchEvent(new h.dom.window.PageTransitionEvent("pageshow",{persisted:true}));await settle();
 h.release(99);await settle();
 assert.notEqual(h.root().querySelector("section").dataset.highWatermark,"99","pre-navigation response cannot replace restored snapshot");
 h.release(3);await settle();
 assert.equal(h.root().querySelector("section").dataset.highWatermark,"3","restored fetch completes after an older request");
 h.close();
}

console.log("workboard: all snapshot, refresh, recovery, focus, history, expiry, transport-health, and navigation checks passed");
