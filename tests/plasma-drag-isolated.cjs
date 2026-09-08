// Called ONLY from the private compositor harness. Real pointer/button events,
// production event handlers, and real Qt preview surfaces; no system-wide input.
const fs=require('node:fs'),path=require('node:path');
const {spawn,execFileSync}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
const wait=ms=>new Promise(r=>setTimeout(r,ms));
function prepare(rootDir,fd) {
    if(process.env.XDG_RUNTIME_DIR!==path.join(rootDir,'runtime'))throw Error('Not isolated');
    const binary=path.join(rootDir,'isolated-input');
    const flags=execFileSync('pkg-config',['--cflags','--libs','Qt6Core','KWaylandClient','wayland-client'],{encoding:'utf8'}).trim().split(/\s+/);
    execFileSync('c++',['-std=c++17','-fPIC',path.join(__dirname,'isolated-input.cpp'),'-o',binary,...flags]);
    const applications=path.join(rootDir,'data/applications');fs.mkdirSync(applications,{recursive:true});
    fs.writeFileSync(path.join(applications,'tilekeep-isolated-input.desktop'),`[Desktop Entry]\nType=Application\nName=Tilekeep isolated input\nExec=${binary}\nNoDisplay=true\nX-KDE-Wayland-Interfaces=org_kde_kwin_fake_input\n`);
    execFileSync('kbuildsycoca6',['--noincremental'],{stdio:['ignore',fd,fd]});
}
module.exports=async({rootDir,kwin,app,dbus,logFile,fd,baseline})=>{
    if(process.env.XDG_RUNTIME_DIR!==path.join(rootDir,'runtime'))throw Error('Not isolated');
    const binary=path.join(rootDir,'isolated-input');
    const input=spawn(binary,[],{env:{...process.env,TILEKEEP_ISOLATED_KWIN_PID:String(kwin.pid)},stdio:['pipe','pipe',fd]});
    let replies=[];input.stdout.on('data',data=>replies.push(...data.toString().trim().split('\n')));
    const reply=async()=>{for(let i=0;i<100;i++){if(replies.length)return replies.shift();if(input.exitCode!==null)throw Error('Private input failed: '+input.exitCode);await wait(20);}throw Error('Private input timed out');};
    const send=async line=>{input.stdin.write(line+'\n');if(await reply()!=='OK')throw Error('Input acknowledgement missing');await wait(100);};
    let loaded=false;
    try {
        if(await reply()!=='READY')throw Error('Input not ready');
        let source=baseline?execFileSync('git',['show','v0.2.6:src/linux/kwin.qml'],{cwd:path.join(__dirname,'..'),encoding:'utf8'}):fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8');
        source=source.replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_DRY_RUN__','false');
        const test=`
 property int dragCase: 0
 property int dragPhase: 0
 property int dragTicks: 0
 property var testA: null
 property var testB: null
 property var sourceRect: null
 property var targetRect: null
 property var fixedRect: null
 property var wantedRect: null
 property string wantedZone: ''
 function dragCheck(ok,message){if(!ok)throw Error(message);}
 Timer {interval:80;running:true;repeat:true;onTriggered:{try{
   if(++root.dragTicks>100)throw Error('native drag timed out '+JSON.stringify({phase:root.dragPhase,case:root.dragCase,preview:root.previewZone,area:root.previewArea,move:root.testA?.move,cursor:Workspace.cursorPos}));
   if(root.currentPlacement||root.placementQueue.length||root.pendingResizeEnds.size)return;
   const m=root.monitors[0];
   if(root.dragPhase===0){
     root.testA=Workspace.stackingOrder.find(w=>String(w.caption)==='Tilekeep isolated A');
     root.testB=Workspace.stackingOrder.find(w=>String(w.caption)==='Tilekeep isolated B');
     root.dragCheck(root.testA&&root.testB,'owned windows missing');
     const a=root.leaf(),b=root.leaf(),empty=root.leaf();root.assign(a,root.testA);root.assign(b,root.testB);
     const rows={kind:'split',axis:'y',ratio:.5,first:a,second:b,parent:null};a.parent=rows;b.parent=rows;
     m.root={kind:'split',axis:'x',ratio:.5,first:rows,second:empty,parent:null};rows.parent=m.root;empty.parent=m.root;
     root.apply();root.dragPhase=1;return;
   }
   if(root.dragPhase===1){
     root.sourceRect=root.windowRect(root.testA);root.fixedRect=root.windowRect(root.testB);
     root.targetRect=root.dragCase<9||root.dragCase===18?root.rects(m).get(root.slotOf(root.testA)[1]):root.rects(m).get(m.root.second);
     const index=root.dragCase%9,x=[.1,.5,.9][index%3],y=[.1,.5,.9][Math.floor(index/3)],r=root.targetRect;
     const p={x:r.x+r.width*x,y:r.y+r.height*y};
     root.wantedZone=root.fittingEmptyZone(root.testA,r,root.emptyZone(r,p));root.wantedRect=root.emptyPart(r,root.wantedZone);
     console.log('TKDRAG READY',JSON.stringify({case:root.dragCase,start:{x:root.sourceRect.x+root.sourceRect.width*.4,y:root.sourceRect.y+12},point:p}));root.dragPhase=2;root.dragTicks=0;return;
   }
   if(root.dragPhase===2){
     if(!root.testA.move||!dragPreview.visible||root.dragTicks<8)return;
     if(root.dragCase===0)dragPreview.contentItem.grabToImage(result=>result.saveToFile('${rootDir}/hover.png'));
     root.dragCheck(root.previewZone===root.wantedZone,'hover zone mismatch '+JSON.stringify({expected:root.wantedZone,actual:root.previewZone}));
     root.dragCheck(root.geometryMatches(root.previewGeometry,root.wantedRect,1),'hover extent mismatch');
     root.dragCheck(root.geometryMatches(root.previewArea,root.targetRect,1),'full-space guides missing');
     root.dragCheck(Workspace.activeWindow===root.testA,'preview stole focus');
     console.log('TKDRAG HOVER',root.dragCase,root.wantedZone);root.dragPhase=3;root.dragTicks=0;return;
   }
   if(root.dragPhase===3){
     if(root.testA.move||root.interactiveWindows.size||root.dragTicks<8)return;
     const expected=root.dragCase===18?root.sourceRect:root.wantedRect;
     root.dragCheck(root.geometryMatches(root.windowRect(root.testA),expected,2),'release mismatch '+JSON.stringify({case:root.dragCase,expected,actual:root.windowRect(root.testA)}));
     root.dragCheck(root.geometryMatches(root.windowRect(root.testB),root.fixedRect,1),'unrelated window moved');
     root.dragCheck(!dragPreview.visible,'preview survived release');
     console.log('TKDRAG PASS',root.dragCase);root.dragCase++;root.dragTicks=0;
     if(root.dragCase===19){console.log('TKDRAG DONE');this.stop();}else root.dragPhase=0;
   }
 }catch(e){console.log('TKDRAG FAIL',String(e));this.stop();}}}
 `;
        const file=path.join(rootDir,'TKISOLATED0.qml');
        fs.writeFileSync(file,source.slice(0,source.lastIndexOf('}'))+test+'}\n');
        loadScript(dbus,file,'tilekeep-native-drag-test');loaded=true;
        const seen=new Set();
        for(let tick=0;tick<1800;tick++) {
            await wait(100);
            if(kwin.exitCode!==null||kwin.signalCode||app.exitCode!==null||app.signalCode)throw Error('Isolated compositor/client exited');
            const lines=fs.readFileSync(logFile,'utf8').split('\n').filter(l=>l.includes('TKDRAG '));
            for(const line of lines) {
                if(seen.has(line))continue;seen.add(line);console.log(line);
                if(line.includes('TKDRAG FAIL')){await wait(300);throw Error('Native drag regression failed');}
                if(line.includes('TKDRAG READY')) {
                    const data=JSON.parse(line.slice(line.indexOf('{')));
                    await send('move '+data.start.x+' '+data.start.y);await send('button 1');
                    await send('move '+(data.start.x+30)+' '+(data.start.y+35));
                    await send('move '+data.point.x+' '+data.point.y);
                } else if(line.includes('TKDRAG HOVER')) {
                    if(line.includes('HOVER 18 ')){await send('key 1 1');await send('key 1 0');}
                    await send('button 0');
                } else if(line.includes('TKDRAG DONE')) {
                    console.log('PASS 18 native drag/hover/drop targets plus Escape; quarter/half/full guides verified');return;
                }
            }
        }
        throw Error('Native test timed out');
    } finally {
        if(input.exitCode===null){input.stdin.end('button 0\nquit\n');await wait(150);if(input.exitCode===null)input.kill('SIGTERM');}
        if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-native-drag-test');
    }
};
module.exports.prepare=prepare;
