/* ButFun — 素材展示 / 可玩切片
   純 canvas，零相依。所有 sprite 由 assets/*.png 載入。 */
(() => {
'use strict';

// ---------- config ----------
const TS = 32, MAP_W = 40, MAP_H = 30;
const WORLD_W = MAP_W * TS, WORLD_H = MAP_H * TS;
const DAY_LEN = 140;           // 秒 / 一整天
const GROW_TIME = 7;           // 秒 / 每階段澆水後成長

// ---------- assets ----------
const SRC = {
  tileset_a:'assets/tileset_a.png', tileset_b:'assets/tileset_b.png', tileset_c:'assets/tileset_c.png',
  field:'assets/field.png', fence:'assets/fence.png', player:'assets/player.png',
  ship:'assets/ship.png', workshop:'assets/workshop.png', tree:'assets/tree.png', rock:'assets/rock.png',
};
const IMG = {};
let loaded = 0; const total = Object.keys(SRC).length;

// ---------- palette tints for night overlay ----------
const PAL = {
  a:{ name:'A · 邊境星', night:'rgba(14,24,52,0.55)', dusk:'rgba(120,70,40,0.18)', amb:'#9bd0ff' },
  b:{ name:'B · 黃昏苔原', night:'rgba(30,28,46,0.5)',  dusk:'rgba(150,90,40,0.28)', amb:'#ffd59b' },
  c:{ name:'C · 乙太霧星', night:'rgba(10,40,60,0.6)',  dusk:'rgba(30,90,110,0.2)',  amb:'#7fe0e6' },
};
let palette = 'a';

// ---------- world state ----------
let tiles = [];            // {b,v}  b:0 grass 1 dirt 2 water 3 stone
let solid = [];            // bool grid
const fields = new Map();  // "x,y" -> {stage,watered,growth}
let statics = [];          // depth-sorted static entities (props/trees/rocks/fence)
const FIELD = {x0:8,y0:12,x1:13,y1:16}; // farmable rect (inclusive)
const GATE = {x:10,y:17};  // gap in bottom fence

const key = (x,y)=>x+','+y;
function rng(seed){ let a=seed; return ()=>{a|=0;a=a+0x6D2B79F5|0;let t=Math.imul(a^a>>>15,1|a);t=t+Math.imul(t^t>>>7,61|t)^t;return((t^t>>>14)>>>0)/4294967296;}; }

function inRect(x,y,r){ return x>=r.x0&&x<=r.x1&&y>=r.y0&&y<=r.y1; }

function buildWorld(){
  const r = rng(1337);
  tiles = []; solid = [];
  for(let y=0;y<MAP_H;y++){ tiles[y]=[]; solid[y]=[];
    for(let x=0;x<MAP_W;x++){ tiles[y][x] = {b:0, v:Math.floor(r()*4)}; solid[y][x]=false; }
  }
  // pond (ellipse)
  const pc={x:6.5,y:22.5,rx:4.2,ry:3.3};
  for(let y=0;y<MAP_H;y++)for(let x=0;x<MAP_W;x++){
    const d=((x-pc.x)/pc.rx)**2 + ((y-pc.y)/pc.ry)**2;
    if(d<=1){ tiles[y][x]={b:2,v:0}; solid[y][x]=true; }
  }
  // stone plaza around workshop
  for(let y=4;y<=10;y++)for(let x=15;x<=24;x++){ tiles[y][x]={b:3,v:Math.floor(r()*4)}; }
  // dirt paths (width 2)
  function paintDirt(x,y){ if(x>=0&&y>=0&&x<MAP_W&&y<MAP_H&&tiles[y][x].b!==2){ tiles[y][x]={b:1,v:Math.floor(r()*4)}; } }
  for(let y=10;y<=24;y++){ paintDirt(19,y); paintDirt(20,y); }      // vertical spine
  for(let x=9;x<=20;x++){ paintDirt(x,19); paintDirt(x,20); }       // horizontal to field
  for(let x=20;x<=27;x++){ paintDirt(x,16); paintDirt(x,17); }      // branch to ship
  // farmable field cells — seeded with a variety of stages so the plot reads
  // as a living farm and showcases every field sprite at a glance
  const seed = [
    [0,1,2,3,4,5],
    [1,2,3,4,5,5],
    [2,3,4,5,0,1],
    [3,4,5,0,1,2],
    [5,5,0,1,2,3],
  ];
  let ry=0;
  for(let y=FIELD.y0;y<=FIELD.y1;y++){ let rx=0;
    for(let x=FIELD.x0;x<=FIELD.x1;x++){
      const st=(seed[ry]&&seed[ry][rx]!=null)?seed[ry][rx]:0;
      fields.set(key(x,y), {stage:st, watered:(st>=2&&st<=4), growth:Math.random()*3});
      rx++;
    } ry++;
  }

  // ----- static entities -----
  statics = [];
  // fence ring around field
  addFence();
  // workshop (anchor bottom-center on plaza)
  addProp('workshop', 19.5*TS, 10*TS, 1.6);
  markSolidRect(18,7,21,9);
  // broken ship
  addProp('ship', 26*TS, 17.2*TS, 1.4);
  markSolidRect(24,15,29,16);
  // trees
  [[3,8],[31,5],[34,19],[5,15],[36,11],[12,25],[29,24],[33,26],[2,18],[37,22]].forEach(([tx,ty])=>{
    addProp('tree', (tx+0.5)*TS, (ty+1)*TS, 1.4); solid[ty][tx]=true;
  });
  // rocks
  [[14,6],[31,13],[8,27],[25,8],[35,16],[16,25]].forEach(([rx,ry])=>{
    addProp('rock', (rx+0.5)*TS, (ry+1)*TS, 1.3); solid[ry][rx]=true;
  });
  // world border solid
  for(let x=0;x<MAP_W;x++){ solid[0][x]=true; solid[MAP_H-1][x]=true; }
  for(let y=0;y<MAP_H;y++){ solid[y][0]=true; solid[y][MAP_W-1]=true; }
}

function markSolidRect(x0,y0,x1,y1){ for(let y=y0;y<=y1;y++)for(let x=x0;x<=x1;x++) if(solid[y]) solid[y][x]=true; }

function addProp(kind, cx, baseY, scale){
  statics.push({ kind, cx, baseY, scale, sway: kind==='tree' ? Math.random()*6.28 : 0 });
}

// fence: rectangle just outside field, gap at gate
const fenceCells = [];
function addFence(){
  const x0=FIELD.x0-1, y0=FIELD.y0-1, x1=FIELD.x1+1, y1=FIELD.y1+1;
  for(let x=x0;x<=x1;x++){ for(let y=y0;y<=y1;y++){
    const edge = (x===x0||x===x1||y===y0||y===y1);
    if(!edge) continue;
    if(y===y1 && (x===GATE.x||x===GATE.x+1)) continue; // gate gap
    let piece=0, rot=0;
    const corner=(x===x0||x===x1)&&(y===y0||y===y1);
    if(corner){ piece=2;
      if(x===x0&&y===y0) rot=0;        // TL : E+S
      else if(x===x1&&y===y0) rot=90;  // TR : S+W
      else if(x===x1&&y===y1) rot=180; // BR
      else rot=270;                    // BL
    } else if(y===y0||y===y1){ piece=0; }
    else { piece=1; }
    fenceCells.push({x,y,piece,rot});
    statics.push({ kind:'fence', cx:x*TS, baseY:(y+1)*TS, piece, rot, tx:x, ty:y });
    if(solid[y]) solid[y][x]=true;
  }}
  // keep gate walkable
  solid[y1][GATE.x]=false; solid[y1][GATE.x+1]=false;
}

// ---------- player & npcs ----------
const player = { x:11.5*TS, y:18.5*TS, dir:0, frame:0, anim:0, moving:false, speed:80, ether:0 };
const NPC_TINT = ['#b5483f','#7a59b0','#3f8f6b','#c98a3f'];
const NPC_NAME = ['歐拉','銅鈴','蕨拾','星砂'];
const NPC_SAY = ['今天乙太回流好旺','幫我看看精煉機?','這片田快熟了','邊境星的黃昏最美','一起修飛船嗎','水波好療癒'];
let npcs = [];
function buildNpcs(){
  npcs = NPC_NAME.map((nm,i)=>({
    name:nm, tint:NPC_TINT[i],
    x:(10+i*4)*TS, y:(20+ (i%2)*3)*TS, dir:0, frame:0, anim:0,
    tx:(10+i*4)*TS, ty:(20+(i%2)*3)*TS, wait:Math.random()*3, speed:42,
    say:'', sayT:0,
  }));
}

// tinted player sheets per npc (recolor cloth/skin subtly toward tint)
const tintCache = {};
function tintedSheet(color){
  if(tintCache[color]) return tintCache[color];
  const c=document.createElement('canvas'); c.width=128;c.height=128;
  const x=c.getContext('2d'); x.imageSmoothingEnabled=false;
  x.drawImage(IMG.player,0,0);
  x.globalCompositeOperation='source-atop';
  x.globalAlpha=0.32; x.fillStyle=color; x.fillRect(0,0,128,128);
  x.globalAlpha=1; x.globalCompositeOperation='source-over';
  tintCache[color]=c; return c;
}

// ---------- input ----------
const keys = {};
addEventListener('keydown', e=>{
  if(['ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(e.key)) e.preventDefault();
  keys[e.key.toLowerCase()]=true;
  if(e.key===' '||e.key.toLowerCase()==='e'){ e.preventDefault(); actOnFront(); }
});
addEventListener('keyup', e=>{ keys[e.key.toLowerCase()]=false; });

let pointer={down:false, sx:0, sy:0, startX:0, startY:0, t0:0, moved:false};
const canvas = document.getElementById('game');
function evtPos(e){ const r=canvas.getBoundingClientRect(); const t=e.touches?e.touches[0]:e;
  return { sx:t.clientX-r.left, sy:t.clientY-r.top }; }
function onDown(e){ const p=evtPos(e); pointer.down=true; pointer.sx=p.sx;pointer.sy=p.sy;
  pointer.startX=p.sx;pointer.startY=p.sy;pointer.t0=performance.now();pointer.moved=false; }
function onMove(e){ if(!pointer.down)return; const p=evtPos(e); pointer.sx=p.sx;pointer.sy=p.sy;
  if(Math.hypot(p.sx-pointer.startX,p.sy-pointer.startY)>10) pointer.moved=true; }
function onUp(e){ if(!pointer.down)return; pointer.down=false;
  const dt=performance.now()-pointer.t0;
  if(!pointer.moved && dt<400){ tapFarm(pointer.sx,pointer.sy); } }
canvas.addEventListener('mousedown',onDown); addEventListener('mousemove',onMove); addEventListener('mouseup',onUp);
canvas.addEventListener('touchstart',e=>{e.preventDefault();onDown(e);},{passive:false});
addEventListener('touchmove',e=>{onMove(e);},{passive:false});
addEventListener('touchend',e=>{onUp(e);});

let hoverTile=null;
canvas.addEventListener('mousemove', e=>{ const p=evtPos(e); hoverTile=screenToTile(p.sx,p.sy); });
canvas.addEventListener('mouseleave', ()=>hoverTile=null);

// ---------- camera ----------
let zoom=3, camX=0, camY=0, VW=0, VH=0;
function resize(){
  VW=innerWidth; VH=innerHeight;
  canvas.width=VW; canvas.height=VH;
  zoom=Math.max(2, Math.min(4, Math.round(VH/(15*TS))));
}
addEventListener('resize', resize);

function screenToTile(sx,sy){
  const wx=(sx)/zoom+camX, wy=(sy)/zoom+camY;
  return { x:Math.floor(wx/TS), y:Math.floor(wy/TS) };
}

// ---------- farming ----------
function nextAction(f){
  if(f.stage===0) return '翻土';
  if(f.stage===1) return '播種';
  if(f.stage>=2 && f.stage<=4 && !f.watered) return '澆水';
  if(f.stage===5) return '收成';
  return null;
}
function doAction(tx,ty){
  const f=fields.get(key(tx,ty)); if(!f) return false;
  // must be near
  const px=player.x/TS, py=player.y/TS;
  if(Math.abs(px-(tx+0.5))>1.7 || Math.abs(py-(ty+0.5))>1.7) { toast('走近一點再操作'); return false; }
  if(f.stage===0){ f.stage=1; toast('翻土完成 — 點一下播種'); }
  else if(f.stage===1){ f.stage=2; f.watered=false; f.growth=0; toast('播下乙太種子 — 記得澆水'); }
  else if(f.stage>=2&&f.stage<=4&&!f.watered){ f.watered=true; toast('澆水完成 — 作物開始成長'); }
  else if(f.stage===5){ const gain=8+Math.floor(Math.random()*8); player.ether+=gain; f.stage=1; f.watered=false; f.growth=0;
    toast('+'+gain+' 乙太 ✦ 收成!'); floatText(tx,ty,'+'+gain+' 乙太'); }
  return true;
}
function tapFarm(sx,sy){ const t=screenToTile(sx,sy); doAction(t.x,t.y); }
function actOnFront(){
  const d=[[0,1],[-1,0],[1,0],[0,-1]][player.dir];
  const tx=Math.floor(player.x/TS)+d[0], ty=Math.floor(player.y/TS)+d[1];
  doAction(tx,ty);
}

// ---------- floating text ----------
const floats=[];
function floatText(tx,ty,txt){ floats.push({x:(tx+0.5)*TS,y:ty*TS,txt,life:1.4}); }

// ---------- toast ----------
let toastMsg='', toastT=0;
function toast(m){ toastMsg=m; toastT=2.6; const el=document.getElementById('toast'); el.textContent=m; el.classList.add('show'); }

// ---------- day/night ----------
let clock=DAY_LEN*0.28; // start mid-morning
function timeStr(){
  const h=Math.floor((clock/DAY_LEN)*24); const m=Math.floor(((clock/DAY_LEN)*24*60)%60);
  let ph='晝'; const t=clock/DAY_LEN;
  if(t<0.22) ph='晨'; else if(t<0.5) ph='晝'; else if(t<0.62) ph='昏'; else if(t<0.78) ph='夜'; else if(t<0.92) ph='夜'; else ph='晨';
  return ph+' '+String(h).padStart(2,'0')+':'+String(m).padStart(2,'0');
}
function nightAlpha(){ // 0 day .. 1 deep night
  const t=clock/DAY_LEN;
  // day 0.2..0.55 bright, night 0.7..0.95
  if(t<0.18) return lerp(0.55,0,t/0.18);
  if(t<0.5) return 0;
  if(t<0.66) return lerp(0,0.85,(t-0.5)/0.16);
  if(t<0.9) return 0.85;
  return lerp(0.85,0.55,(t-0.9)/0.1);
}
const lerp=(a,b,t)=>a+(b-a)*Math.max(0,Math.min(1,t));

// ---------- update ----------
let last=0, FRAMES=0;
function update(dt){
  FRAMES++;
  // clock
  clock=(clock+dt)%DAY_LEN;
  // player movement
  let mx=0,my=0;
  if(keys['arrowup']||keys['w']) my-=1;
  if(keys['arrowdown']||keys['s']) my+=1;
  if(keys['arrowleft']||keys['a']) mx-=1;
  if(keys['arrowright']||keys['d']) mx+=1;
  // pointer joystick (held & dragged)
  if(pointer.down && pointer.moved){
    const px=(player.x-camX)*zoom, py=(player.y-camY)*zoom;
    const dx=pointer.sx-px, dy=pointer.sy-py;
    if(Math.hypot(dx,dy)>14){ mx=dx; my=dy; }
  }
  const len=Math.hypot(mx,my);
  player.moving = len>0.01;
  if(player.moving){
    mx/=len; my/=len;
    if(Math.abs(mx)>Math.abs(my)) player.dir = mx<0?1:2; else player.dir = my<0?3:0;
    moveEntity(player, mx*player.speed*dt, my*player.speed*dt);
    player.anim+=dt*8; player.frame=Math.floor(player.anim)%4;
  } else { player.frame=0; player.anim=0; }

  // npcs
  npcs.forEach(n=>{
    n.wait-=dt;
    if(n.wait<=0){
      // pick new target near current within walkable grass/dirt
      const tx=4+Math.floor(Math.random()*32), ty=4+Math.floor(Math.random()*22);
      if(!solid[ty]||!solid[ty][tx]){ n.tx=(tx+0.5)*TS; n.ty=(ty+0.5)*TS; }
      n.wait=2+Math.random()*4;
      if(Math.random()<0.5){ n.say=NPC_SAY[Math.floor(Math.random()*NPC_SAY.length)]; n.sayT=3.2; pushChat(n.name,n.say); }
    }
    const dx=n.tx-n.x, dy=n.ty-n.y, d=Math.hypot(dx,dy);
    if(d>2){ const ux=dx/d, uy=dy/d;
      if(Math.abs(ux)>Math.abs(uy)) n.dir=ux<0?1:2; else n.dir=uy<0?3:0;
      moveEntity(n, ux*n.speed*dt, uy*n.speed*dt);
      n.anim+=dt*7; n.frame=Math.floor(n.anim)%4;
    } else { n.frame=0; }
    if(n.sayT>0) n.sayT-=dt;
  });

  // crop growth
  fields.forEach(f=>{
    if(f.stage>=2&&f.stage<=4&&f.watered){
      f.growth+=dt;
      if(f.growth>=GROW_TIME){ f.growth=0; f.stage++; f.watered=false; }
    }
  });

  // floats
  for(let i=floats.length-1;i>=0;i--){ floats[i].y-=dt*18; floats[i].life-=dt; if(floats[i].life<=0) floats.splice(i,1); }
  if(toastT>0){ toastT-=dt; if(toastT<=0) document.getElementById('toast').classList.remove('show'); }

  // camera follow
  camX=Math.max(0, Math.min(WORLD_W-VW/zoom, player.x-VW/zoom/2));
  camY=Math.max(0, Math.min(WORLD_H-VH/zoom, player.y-VH/zoom/2));
}

function moveEntity(ent, dx, dy){
  const nx=ent.x+dx, ny=ent.y+dy;
  if(!collide(nx, ent.y)) ent.x=nx;
  if(!collide(ent.x, ny)) ent.y=ny;
}
function collide(px,py){
  // feet hitbox
  const fx=px, fy=py+10;
  const tx=Math.floor(fx/TS), ty=Math.floor(fy/TS);
  if(tx<0||ty<0||tx>=MAP_W||ty>=MAP_H) return true;
  return !!(solid[ty]&&solid[ty][tx]);
}

// ---------- render ----------
const ctx = canvas.getContext('2d');
function tilesetImg(){ return IMG['tileset_'+palette]; }

function render(){
  ctx.imageSmoothingEnabled=false;
  ctx.setTransform(1,0,0,1,0,0);
  ctx.fillStyle='#0b0d12'; ctx.fillRect(0,0,VW,VH);
  ctx.setTransform(zoom,0,0,zoom,-camX*zoom,-camY*zoom);

  const x0=Math.max(0,Math.floor(camX/TS)), y0=Math.max(0,Math.floor(camY/TS));
  const x1=Math.min(MAP_W-1,Math.ceil((camX+VW/zoom)/TS)), y1=Math.min(MAP_H-1,Math.ceil((camY+VH/zoom)/TS));
  const wframe=Math.floor(performance.now()/170)%4;
  const ts=tilesetImg();

  // ground
  for(let y=y0;y<=y1;y++)for(let x=x0;x<=x1;x++){
    const t=tiles[y][x]; let sx,sy;
    if(t.b===2){ sx=wframe*TS; sy=2*TS; }
    else { sx=t.v*TS; sy=t.b===0?0:(t.b===1?TS:3*TS); }
    ctx.drawImage(ts, sx,sy,TS,TS, x*TS,y*TS,TS,TS);
  }
  // field sprites
  fields.forEach((f,k)=>{
    const [fx,fy]=k.split(',').map(Number);
    if(fx<x0-1||fx>x1+1||fy<y0-1||fy>y1+1) return;
    let col=0;
    if(f.stage===0) col=0;
    else if(f.stage===1) col=f.watered?2:1;
    else col=[null,null,3,4,5,6][f.stage];
    ctx.drawImage(IMG.field, col*TS,0,TS,TS, fx*TS,fy*TS,TS,TS);
    // dry hint when crop needs water
    if(f.stage>=2&&f.stage<=4&&!f.watered){ ctx.drawImage(IMG.field, 7*TS,0,TS,TS, fx*TS,fy*TS,TS,TS); }
  });

  // hover highlight on farmable
  if(hoverTile){ const f=fields.get(key(hoverTile.x,hoverTile.y));
    if(f){ ctx.strokeStyle='rgba(232,224,207,0.85)'; ctx.lineWidth=1; ctx.strokeRect(hoverTile.x*TS+0.5,hoverTile.y*TS+0.5,TS-1,TS-1);
      const a=nextAction(f); if(a) labelAt((hoverTile.x+0.5)*TS, hoverTile.y*TS-2, a); } }

  // entities (statics + player + npcs) depth-sorted
  const ents=[];
  for(const s of statics){ if(s.cx< camX-80||s.cx>camX+VW/zoom+80) continue; ents.push(s); }
  ents.push({kind:'player', cx:player.x, baseY:player.y+14});
  for(const n of npcs) ents.push({kind:'npc', ref:n, cx:n.x, baseY:n.y+14});
  ents.sort((a,b)=>a.baseY-b.baseY);
  for(const e of ents) drawEntity(e);

  // glowing crops (additive) — stronger at night
  const na=nightAlpha();
  ctx.globalCompositeOperation='lighter';
  fields.forEach((f,k)=>{ if(f.stage!==5) return; const [fx,fy]=k.split(',').map(Number);
    const cx=(fx+0.5)*TS, cy=fy*TS+12; const pulse=0.5+0.5*Math.sin(performance.now()/400+fx);
    const rad=10+pulse*3; const g=ctx.createRadialGradient(cx,cy,0,cx,cy,rad);
    const a=(0.35+na*0.5); g.addColorStop(0,`rgba(240,215,122,${a})`); g.addColorStop(1,'rgba(240,215,122,0)');
    ctx.fillStyle=g; ctx.fillRect(cx-rad,cy-rad,rad*2,rad*2);
  });
  ctx.globalCompositeOperation='source-over';

  // floating texts
  ctx.font='6px monospace'; ctx.textAlign='center';
  floats.forEach(f=>{ ctx.globalAlpha=Math.min(1,f.life); ctx.fillStyle='#0b0d12'; ctx.fillText(f.txt,f.x+0.5,f.y+0.5);
    ctx.fillStyle='#f0d77a'; ctx.fillText(f.txt,f.x,f.y); ctx.globalAlpha=1; });
  ctx.textAlign='left';

  // ---- day/night overlay (screen space) ----
  ctx.setTransform(1,0,0,1,0,0);
  if(na>0.001){ const p=PAL[palette]; ctx.fillStyle=p.night.replace(/[\d.]+\)$/,(na*0.85).toFixed(2)+')');
    ctx.fillRect(0,0,VW,VH);
    // vignette
    const vg=ctx.createRadialGradient(VW/2,VH/2,Math.min(VW,VH)*0.3,VW/2,VH/2,Math.max(VW,VH)*0.7);
    vg.addColorStop(0,'rgba(0,0,0,0)'); vg.addColorStop(1,`rgba(0,0,0,${0.4*na})`);
    ctx.fillStyle=vg; ctx.fillRect(0,0,VW,VH);
  }
  // dusk warm wash
  const t=clock/DAY_LEN;
  if(t>0.46&&t<0.64){ const dk=Math.sin((t-0.46)/0.18*Math.PI); ctx.fillStyle=PAL[palette].dusk.replace(/[\d.]+\)$/,(dk*0.3).toFixed(2)+')'); ctx.fillRect(0,0,VW,VH); }
}

function labelAt(wx,wy,txt){
  ctx.font='6px monospace'; ctx.textAlign='center';
  const w=txt.length*6+6;
  ctx.fillStyle='rgba(11,13,18,0.8)'; ctx.fillRect(wx-w/2,wy-9,w,9);
  ctx.fillStyle='#c9a24b'; ctx.fillText(txt,wx,wy-2.5); ctx.textAlign='left';
}

function drawEntity(e){
  if(e.kind==='player'){ drawChar(IMG.player, player); return; }
  if(e.kind==='npc'){ drawChar(tintedSheet(e.ref.tint), e.ref, e.ref.name, e.ref.say&&e.ref.sayT>0?e.ref.say:''); return; }
  if(e.kind==='fence'){ drawFence(e); return; }
  // props
  const img=IMG[e.kind]; if(!img) return;
  const w=img.width*e.scale/ (e.kind==='tree'?1:1);
  const dw=img.width*e.scale, dh=img.height*e.scale;
  let skew=0;
  if(e.kind==='tree'){ skew=Math.sin(performance.now()/900+e.sway)*1.2; }
  ctx.drawImage(img, Math.round(e.cx-dw/2+skew*0.3), Math.round(e.baseY-dh), dw, dh);
}

function drawFence(e){
  ctx.save();
  ctx.translate(e.tx*TS+TS/2, e.ty*TS+TS/2);
  ctx.rotate(e.rot*Math.PI/180);
  ctx.drawImage(IMG.fence, e.piece*TS,0,TS,TS, -TS/2,-TS/2,TS,TS);
  ctx.restore();
}

function drawChar(sheet, ent, name, say){
  const sx=ent.frame*TS, sy=ent.dir*TS;
  ctx.drawImage(sheet, sx,sy,TS,TS, Math.round(ent.x-TS/2), Math.round(ent.y-TS+6), TS,TS);
  if(name){ ctx.font='5px monospace'; ctx.textAlign='center';
    ctx.fillStyle='rgba(11,13,18,0.7)'; const w=name.length*5+4; ctx.fillRect(ent.x-w/2, ent.y-TS-2, w,7);
    ctx.fillStyle='#e8e0cf'; ctx.fillText(name, ent.x, ent.y-TS+3); ctx.textAlign='left'; }
  if(say){ ctx.font='5px monospace'; ctx.textAlign='center';
    const w=say.length*5+6; const by=ent.y-TS-12;
    ctx.fillStyle='rgba(232,224,207,0.95)'; roundRect(ent.x-w/2,by-7,w,9,2); ctx.fill();
    ctx.fillStyle='#0b0d12'; ctx.fillText(say,ent.x,by); ctx.textAlign='left'; }
}
function roundRect(x,y,w,h,r){ ctx.beginPath(); ctx.moveTo(x+r,y); ctx.arcTo(x+w,y,x+w,y+h,r); ctx.arcTo(x+w,y+h,x,y+h,r); ctx.arcTo(x,y+h,x,y,r); ctx.arcTo(x,y,x+w,y,r); ctx.closePath(); }

// ---------- HUD ----------
function updateHud(){
  document.getElementById('hudPlayers').textContent='線上：'+(npcs.length+1);
  document.getElementById('hudEther').textContent='乙太：'+player.ether;
  document.getElementById('hudTime').textContent=timeStr();
}
const chatLog=[];
function pushChat(who,msg){ chatLog.push({who,msg}); if(chatLog.length>6) chatLog.shift();
  const el=document.getElementById('chatLog'); el.innerHTML=chatLog.map(c=>`<div><span class="who">${c.who}</span>：${c.msg}</div>`).join('');
  el.style.display='block'; }

// ---------- loop ----------
function frame(ts){ const dt=Math.min(0.05,(ts-last)/1000||0); last=ts;
  try { update(dt); render(); updateHud(); }
  catch(err){ console.error('BF frame error:', err); window.__bfError=(err&&err.stack)||String(err); }
  requestAnimationFrame(frame); }

// ---------- boot ----------
function boot(){
  resize(); buildWorld(); buildNpcs();
  // warm chat
  pushChat('系統','歡迎回到邊境星 ✦ 走到田邊點一下開始耕作');
  requestAnimationFrame(frame);
}
function tryStart(){ loaded++; if(loaded>=total) boot(); }
for(const k in SRC){ const im=new Image(); im.onload=tryStart; im.onerror=tryStart; im.src=SRC[k]; IMG[k]=im; }

// ---------- expose for UI ----------
window.BF = {
  setPalette(p){ palette=p; document.querySelectorAll('.palbtn').forEach(b=>b.classList.toggle('on',b.dataset.p===p)); },
  setTime(frac){ clock=frac*DAY_LEN; },
  getState(){ return {ether:player.ether, time:timeStr(), px:(player.x/TS).toFixed(1), py:(player.y/TS).toFixed(1), frames:FRAMES, err:window.__bfError||'none'}; },
  tillAll(){ fields.forEach(f=>{ if(f.stage===0) f.stage=1; }); },
  sim(steps, dt){ steps=steps||120; dt=dt||0.016; for(let i=0;i<steps;i++) update(dt); render(); updateHud(); },
  setKey(k,v){ keys[k]=v; },
  act(tx,ty){ return doAction(tx,ty); },
};
})();
