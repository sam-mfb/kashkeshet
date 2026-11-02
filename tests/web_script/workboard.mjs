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
  const listeners={},timers=[]; let requests=[],next=content(1),calls=0,hold=false;
  const dom=new JSDOM(html.replace(/data-high-watermark="[0-9]+"/g,'data-high-watermark="0"').replace(/data-evaluated-at="[0-9]+"/g,'data-evaluated-at="0"'),{url:base+"/channels/"+channel+"/work",runScripts:"dangerously",pretendToBeVisual:true,beforeParse(w){
    w.scrollTo=()=>{};
    w.setInterval=(fn,ms)=>{timers.push({fn,ms});return timers.length;};
    w.clearInterval=()=>{};
    w.fetch=async()=>{calls++;if(hold)return await new Promise(resolve=>requests.push(resolve));return{ok:true,redirected:false,text:async()=>next};};
    w.EventSource=class {
      constructor(){this.readyState=1;}
      addEventListener(name,fn){(listeners[name]??=[]).push(fn);}
      close(){}
    };
  }});
  return {
    dom,root:()=>dom.window.document.getElementById("workboard"),calls:()=>calls,timers,
    event:(name,data={})=>{for(const fn of listeners[name]??[])fn({data:JSON.stringify(data)});},
    next:(n,label)=>{next=content(n,label);},
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

console.log("workboard: all snapshot, refresh, recovery, focus, history, expiry, and transport-health checks passed");
