// UI acceptance against the simulated preview only.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
const base=process.argv[2];
assert.equal(new URL(base).hostname,"127.0.0.1");
const { chromium } = await import(pathToFileURL(resolve(process.env.KKT_PLAYWRIGHT_MODULE)).href);
const output=resolve("target/k1-validation/screenshots");
await mkdir(output,{recursive:true});
const browser=await chromium.launch({headless:true,args:["--no-sandbox"]});
try {
 const page=await browser.newPage({viewport:{width:1440,height:1100}});
 const home=await(await fetch(base)).text();
 const channels=[...new Set([...home.matchAll(/href="\/channels\/(ch_[a-z0-9]+)"/g)].map(m=>m[1]))];
 assert.ok(channels.length>1);
 const aggregate=base+"/work?channels="+channels.join(",");
 const measurements=[];
 for(const width of [1440,390]){
  await page.setViewportSize({width,height:width===390?844:1100});
  await page.goto(aggregate);
  await page.waitForSelector(".work-state");
  const geometry=await page.evaluate(()=>({
   overflow:document.documentElement.scrollWidth>innerWidth,
   overflowing:[...document.querySelectorAll("body *")].filter(n=>n.getBoundingClientRect().right>innerWidth).map(n=>({tag:n.tagName,cls:n.className,text:n.textContent.slice(0,90),right:n.getBoundingClientRect().right})),
   rows:[...document.querySelectorAll(".work-item")].filter(n=>n.getBoundingClientRect().height).map(n=>({
    status:n.querySelector(".work-state").getBoundingClientRect().left,
    context:n.querySelector(".work-context").getBoundingClientRect().left
   }))
  }));
  assert.ok(geometry.rows.length>=3);
  for(const column of ["status","context"]){
   const spread=Math.max(...geometry.rows.map(r=>r[column]))-Math.min(...geometry.rows.map(r=>r[column]));
   assert.ok(spread<=1, width+"px "+column+" column differs by "+spread+"px");
  }
  assert.equal(geometry.overflow,false,width+"px horizontal overflow: "+JSON.stringify(geometry.overflowing));
  measurements.push({width,...geometry});
  if(width===1440)await page.screenshot({path:output+"/aggregate-work.png",fullPage:true});
 }
 await page.setViewportSize({width:1440,height:1100});
 await page.goto(aggregate);
 const channel=await page.locator(".work-inbox [data-home-channel]").first().getAttribute("data-home-channel");
 await page.goto(base+"/channels/"+channel+"/work");
 await page.screenshot({path:output+"/channel-work.png",fullPage:true});
 const request=page.locator(".work-inbox details").first();
 await request.evaluate(n=>n.open=true);
 await page.screenshot({path:output+"/request-details.png",fullPage:true});
 await request.evaluate(n=>n.open=false);
 await page.setViewportSize({width:390,height:844});
 await page.screenshot({path:output+"/channel-work-mobile.png",fullPage:true});
 await writeFile(resolve("target/k1-validation/k2-layout.json"),JSON.stringify(measurements,null,2)+"\n");
 console.log(JSON.stringify(measurements));
} finally {await browser.close();}
