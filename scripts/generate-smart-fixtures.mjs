// Development-only golden generator. Formal Rust detection/tests never run JS.
// All inputs are public, synthetic pixel patterns; no screenshots/fonts/UI trees.
// Usage: node scripts/generate-smart-fixtures.mjs [read-only JS reference checkout]
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';
import { createHash } from 'node:crypto';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
if (!process.argv[2]) throw new Error('Pass a read-only dsh-image-feedback checkout pinned to 6f256a67ecfc043dacd2614382d9c19f2437f66d.');
const reference=resolve(process.argv[2]);
const source=await readFile(join(reference,'src/smart-selection.js'),'utf8');
const {createSmartSelectionEngine}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const engine=createSmartSelectionEngine();
// Reuse only the public test fixture drawing functions, not test execution. This
// ensures these goldens exercise exactly the reference AA/CJK/layout rasters.
const basic=await readFile(join(reference,'tests/smart-selection.test.mjs'),'utf8');
const helpers=basic.slice(basic.indexOf('function rgba('),basic.indexOf('function near('))+
  basic.slice(basic.indexOf('function random('),basic.indexOf("test('light and dark"));
const g=Function(helpers+';return {rgba,pixel,box,bordered,text,antialiasedBox,paintShape,commonShapes,random};')();
const layoutSource=await readFile(join(reference,'tests/smart-selection-layout.test.mjs'),'utf8');
const l=Function(layoutSource.slice(layoutSource.indexOf('const clamp ='),layoutSource.indexOf('function pickAt('))+
  ';return {rgba,box,line,scaled,fixture};')();
function crc32(data) { let c=0xffffffff; for(const b of data){c^=b;for(let k=0;k<8;k++)c=(c>>>1)^((c&1)?0xedb88320:0);}return (c^0xffffffff)>>>0; }
function chunk(name,data){const tag=Buffer.from(name),n=Buffer.alloc(4),crc=Buffer.alloc(4);n.writeUInt32BE(data.length);crc.writeUInt32BE(crc32(Buffer.concat([tag,data])));return Buffer.concat([n,tag,data,crc]);}
function png(image){const header=Buffer.alloc(13);header.writeUInt32BE(image.width);header.writeUInt32BE(image.height,4);header[8]=8;header[9]=6;
 const rows=Buffer.alloc((image.width*4+1)*image.height);for(let y=0;y<image.height;y++)Buffer.from(image.data.buffer,image.data.byteOffset+y*image.width*4,image.width*4).copy(rows,y*(image.width*4+1)+1);
 return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(rows)),chunk('IEND',Buffer.alloc(0))]);}
const cases=[];const out=join(root,'tests/fixtures');await mkdir(out,{recursive:true});
async function add(name,image,scale=1,points=[]){
 const file=`smart-${name}.png`;await writeFile(join(out,file),png(image));const candidates=engine.detect(image,{scale});
 const probes=points.map(point=>({point,expected:engine.pick(candidates,point)?.id??null}));
 // Every detected item adds a center and an exterior-corner probe, including
 // previous-ID hysteresis. These expectations are emitted by JS, not by Rust.
 for(const c of candidates.slice(0,400)){for(const point of [{x:c.rect.x+c.rect.width/2,y:c.rect.y+c.rect.height/2},{x:c.rect.x+0.5,y:c.rect.y+0.5}])probes.push({point,previous:c.id,expected:engine.pick(candidates,point,c)?.id??null});}
 cases.push({name,file,width:image.width,height:image.height,scale,candidates,probes});
}
for(const s of g.commonShapes)for(const dark of [false,true])for(const stroke of [false,true]){
 const image=g.rgba(260,180,dark?[24,27,32,255]:[255,255,255,255]),r={x:64,y:52,width:s.width,height:s.height};
 g.paintShape(image,r,s,dark?[100,157,238,255]:[37,105,210,255],stroke?{stroke:1}:{});
 g.text(image,r.x+Math.floor((r.width-19)/2),r.y+Math.floor((r.height-7)/2),3,stroke?(dark?[220,230,245,255]:[40,45,55,255]):[255,255,255,255]);
 await add(`${s.name}-${dark?'dark':'light'}-${stroke?'stroke':'fill'}`,image);
}
const variants=[
 {name:'capsule',type:'capsule',width:42,height:110},
 {name:'triangle',type:'polygon',vertices:[[0,0],[1,0],[.5,1]]},
 {name:'triangle',type:'polygon',vertices:[[0,.5],[1,0],[1,1]]},
 {name:'triangle',type:'polygon',vertices:[[0,0],[1,.5],[0,1]]},
 {name:'hexagon',type:'polygon',vertices:[[.5,0],[1,.25],[1,.75],[.5,1],[0,.75],[0,.25]]},
 {name:'trapezoid',type:'polygon',vertices:[[0,0],[1,0],[.75,1],[.25,1]]},
 {name:'trapezoid',type:'polygon',vertices:[[.15,0],[.85,0],[1,1],[0,1]]},
 {name:'parallelogram',type:'polygon',vertices:[[0,0],[.75,0],[1,1],[.25,1]]},
 {name:'parallelogram',type:'polygon',vertices:[[.15,0],[1,0],[.85,1],[0,1]]}];
for(const [i,s]of variants.entries()){const image=g.rgba(240,210);g.paintShape(image,{x:67,y:45,width:s.width||86,height:s.height||68},s,[38,104,209,255],{stroke:1});await add(`orientation-${i}`,image);}
for(const dark of [false,true])for(const scale of [1,.7]){
 const {image}=l.fixture(dark);await add(`layout-${dark?'dark':'light'}-${scale}`,scale===1?image:l.scaled(image,scale),scale,
 [{x:90,y:140},{x:215,y:140},{x:245,y:140},{x:90,y:284},{x:245,y:284},{x:430,y:112},{x:430,y:140},{x:1010,y:140},{x:430,y:232},{x:390,y:260},{x:1010,y:260},{x:1000,y:190}].map(p=>({x:p.x*scale,y:p.y*scale})));
}
{const {image}=l.fixture();l.box(image,880,128,100,28,[30,100,210,255]);l.line(image,899,137,5,[255,255,255,255],{small:true});await add('child-button',image,1,[{x:930,y:142},{x:1010,y:140}]);}
for(const dark of [false,true]){
 const bg=dark?[26,29,35,255]:[255,255,255,255],image=g.rgba(400,300,bg);
 g.bordered(image,{x:49,y:38,width:278,height:211},dark?[56,60,68,255]:[221,224,228,255],bg,9);await add(`thin-card-${dark}`,image);
 const text=g.rgba(420,130,dark?[22,24,29,255]:[255,255,255,255]);g.text(text,47,53,12,dark?[226,231,238,255]:[40,45,52,255],2);await add(`line-${dark}`,text);
 const rounded=g.rgba(360,180,dark?[23,26,32,255]:[250,250,250,255]);g.box(rounded,57,63,146,42,dark?[67,117,205,255]:[24,93,207,255],10);g.text(rounded,77,77,7,[255,255,255,255],2);await add(`rounded-${dark}`,rounded);
 const avatar=g.rgba(330,190,dark?[23,26,32,255]:[255,255,255,255]),rng=g.random(2468);
 g.paintShape(avatar,{x:98,y:54,width:64,height:64},{type:'ellipse'},[0,0,0,255],{texture:()=>[35+rng()%170,45+rng()%155,40+rng()%165,255]});await add(`avatar-${dark}`,avatar);
}
{const image=g.rgba(480,360,[247,248,250,255]);g.bordered(image,{x:42,y:35,width:370,height:278},[214,218,224,255],[255,255,255,255],12);g.text(image,72,70,11,[30,37,47,255],2);g.text(image,72,112,14,[94,99,108,255]);g.box(image,221,234,145,44,[35,104,220,255],8);g.text(image,244,249,7,[255,255,255,255],2);await add('nested',image);}
for(const [name,color]of [['blank-light',[255,255,255,255]],['blank-dark',[20,22,25,255]],['hidden-rgb',[50,100,150,0]]])await add(name,g.rgba(200,140,color));
{const image=g.rgba(240,180),rng=g.random(123);for(let i=0;i<image.data.length;i+=4){image.data[i]=rng();image.data[i+1]=rng();image.data[i+2]=rng();}await add('noise',image);}
{const image=g.rgba(320,200);for(let y=0;y<image.height;y++)for(let x=0;x<image.width;x++)g.pixel(image,x,y,[Math.round(90+60*Math.sin(x/30)*Math.cos(y/21)),Math.round(110+x*.2+30*Math.sin(y/13)),Math.round(70+y*.5),255]);await add('smooth-photo',image);}
{const image=g.rgba(280,180);g.paintShape(image,{x:72,y:48,width:74,height:74},{type:'ellipse'},[30,95,205,255],{stroke:2,arcGap:[Math.PI*.15,Math.PI*.65]});await add('open-arc',image);}
{const image=g.rgba(280,180);g.box(image,70,55,90,2,[35,90,200,255]);g.box(image,70,55,2,68,[35,90,200,255]);g.box(image,158,55,2,68,[35,90,200,255]);await add('open-box',image);}
{const image=g.rgba(260,190),r={x:70,y:48,width:88,height:72},color=[40,95,185,255];g.paintShape(image,r,g.commonShapes.find(s=>s.name==='triangle'),color,{stroke:1.5});for(const [x,y]of [[69,119],[71,120],[158,119],[156,120]])g.pixel(image,x,y,color);await add('acute-spurs',image);}
{const image=l.rgba(1280,800);for(let y=4;y<400;y+=20)for(let x=4;x<1260;x+=20)l.box(image,x,y,16,16,[35,105,215,255]);l.line(image,100,650,50,[20,23,28,255],{mixed:true});l.line(image,100,678,12,[20,23,28,255]);await add('dense-layout',image,1,[{x:180,y:657},{x:760,y:685}]);}
{const image=g.rgba(1280,800);for(let y=3;y+16<800;y+=20)for(let x=3;x+16<1280;x+=20)g.box(image,x,y,16,16,[30,100,210,255]);await add('candidate-cap',image);}
const manifest={schema:'smart-selection-parity-1',referenceCommit:'6f256a6',referenceSha256:createHash('sha256').update(source).digest('hex'),provenance:'Public synthetic rasters from reference geometry/layout test helpers; no private screenshots.',cases};
await writeFile(join(out,'smart-goldens.json'),JSON.stringify(manifest,null,2)+'\n');
console.log(`${cases.length} synthetic PNGs; ${cases.reduce((n,c)=>n+c.candidates.length,0)} candidates; ${cases.reduce((n,c)=>n+c.probes.length,0)} pick probes`);
