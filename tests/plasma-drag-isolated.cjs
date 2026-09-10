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
    const controlDrag=process.argv.includes('--control-drag');
    const effectLoaded=name=>dbus('/Effects','org.kde.kwin.Effects.isEffectLoaded',name)==='true';
    let loaded=false,probeLoaded=false;
    try {
        if(await reply()!=='READY')throw Error('Input not ready');
        if(controlDrag) {
            if(effectLoaded('tilekeep-control-marker'))throw Error('Ctrl marker started loaded');
            await send('key 29 1');
            if(effectLoaded('tilekeep-control-marker'))throw Error('ordinary Ctrl press loaded the drag-only marker');
            await send('key 29 0');
            if(effectLoaded('tilekeep-control-marker'))throw Error('Ctrl release did not unload the marker effect');
            console.log('PASS ordinary Ctrl stays inert outside a window drag');

            // Exercise KWin's real interactive-resize state before loading the
            // Tilekeep script. This catches observer regressions independently
            // of the mocked handler tests below.
            const probe=path.join(rootDir,'resize-marker-probe.qml');
            fs.writeFileSync(probe,`import QtQuick\nimport org.kde.kwin 3.0\nItem { Component.onCompleted: { const w=Workspace.stackingOrder.find(w=>String(w.caption)==='Tilekeep isolated A'); if(!w) console.log('TKRESIZEPROBE FAIL'); else { Workspace.activeWindow=w; console.log('TKRESIZEPROBE READY'); } } }\n`);
            loadScript(dbus,probe,'tilekeep-resize-marker-probe');probeLoaded=true;
            let probeReady=false;
            for(let i=0;i<50;i++) {
                await wait(50);
                const log=fs.readFileSync(logFile,'utf8');
                if(log.includes('TKRESIZEPROBE FAIL'))throw Error('Could not activate private resize probe window');
                if(log.includes('TKRESIZEPROBE READY')){probeReady=true;break;}
            }
            if(!probeReady)throw Error('Private resize probe timed out');
            execFileSync('qdbus6',['org.kde.kglobalaccel','/component/kwin','org.kde.kglobalaccel.Component.invokeShortcut','Window Resize'],{stdio:['ignore',fd,fd]});
            await wait(150);
            await send('key 29 1');
            if(!effectLoaded('tilekeep-control-marker'))throw Error('Ctrl resize did not load the marker effect');
            await send('key 29 0');
            if(effectLoaded('tilekeep-control-marker'))throw Error('Ctrl resize marker survived Ctrl release');
            await send('key 1 1');await send('key 1 0');
            console.log('PASS Ctrl is observed during a real private KWin edge resize');
            dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-resize-marker-probe');probeLoaded=false;
        }
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
     root.floating.delete(root.testA);root.floating.delete(root.testB);
     const a=root.leaf(),b=root.leaf(),empty=root.leaf();root.assign(a,root.testA);root.assign(b,root.testB);
     if(root.dragCase===28) {
       m.root={kind:'split',axis:'x',ratio:.25,first:a,second:b,parent:null};a.parent=m.root;b.parent=m.root;
     } else {
       const rows={kind:'split',axis:'y',ratio:.5,first:a,second:b,parent:null};a.parent=rows;b.parent=rows;
       m.root={kind:'split',axis:'x',ratio:.5,first:rows,second:empty,parent:null};rows.parent=m.root;empty.parent=m.root;
     }
     root.apply();root.dragPhase=1;return;
   }
   if(root.dragPhase===1){
     root.sourceRect=root.windowRect(root.testA);root.fixedRect=root.windowRect(root.testB);
     root.targetRect=root.dragCase===28?root.rects(m).get(root.slotOf(root.testB)[1]):root.dragCase<9||root.dragCase===18||(root.dragCase>=19&&root.dragCase<23)?root.rects(m).get(root.slotOf(root.testA)[1]):root.rects(m).get(m.root.second);
     const index=root.dragCase%9,r=root.targetRect;
     const x=root.dragCase===27?.5:root.dragCase===28?.95:root.dragCase>=19?[.2,.8][(root.dragCase-19)%2]:[.1,.5,.9][index%3];
     const y=root.dragCase>=27?.5:root.dragCase>=19?[.2,.8][Math.floor((root.dragCase-19)%4/2)]:[.1,.5,.9][Math.floor(index/3)];
     const p={x:r.x+r.width*x,y:r.y+r.height*y};
     if(root.dragCase>=27) {
       const plan=root.freeDropPreview(root.testA,p);root.dragCheck(!!plan,'free preview plan missing');
       root.wantedZone=plan.zone;root.wantedRect=plan.rect;
     } else {root.wantedZone=root.dragCase>=19?'center':root.fittingEmptyZone(root.testA,r,root.emptyZone(r,p));root.wantedRect=root.emptyPart(r,root.wantedZone);}
     console.log('TKDRAG READY',JSON.stringify({case:root.dragCase,start:{x:root.sourceRect.x+root.sourceRect.width*.4,y:root.sourceRect.y+12},point:p}));root.dragPhase=2;root.dragTicks=0;return;
   }
   if(root.dragPhase===2){
     if(!root.testA.move||!dragPreview.visible||root.dragTicks<8)return;
     if(root.dragCase===0)dragPreview.contentItem.grabToImage(result=>result.saveToFile('${rootDir}/hover.png'));
     if(root.dragCase===19)dragPreview.contentItem.grabToImage(result=>result.saveToFile('${rootDir}/hover-full.png'));
     root.dragCheck(root.previewZone===root.wantedZone,'hover zone mismatch '+JSON.stringify({expected:root.wantedZone,actual:root.previewZone}));
     root.dragCheck(root.geometryMatches(root.previewGeometry,root.wantedRect,1),'hover extent mismatch');
     if(root.dragCase<27)root.dragCheck(root.geometryMatches(root.previewArea,root.targetRect,1),'full-space guides missing');
     else root.dragCheck(root.previewFree,'Ctrl drag was not shown as free');
     root.dragCheck(Workspace.activeWindow===root.testA,'preview stole focus');
     console.log('TKDRAG HOVER',root.dragCase,root.wantedZone);root.dragPhase=3;root.dragTicks=0;return;
   }
   if(root.dragPhase===3){
     if(root.testA.move||root.interactiveWindows.size||root.dragTicks<8)return;
     const expected=root.dragCase===18?root.sourceRect:root.wantedRect;
     root.dragCheck(root.geometryMatches(root.windowRect(root.testA),expected,2),'release mismatch '+JSON.stringify({case:root.dragCase,expected,actual:root.windowRect(root.testA)}));
     if(root.dragCase<27)root.dragCheck(root.geometryMatches(root.windowRect(root.testB),root.fixedRect,1),'unrelated window moved');
     if(root.dragCase===27)root.dragCheck(root.area(root.windowRect(root.testB))>root.area(root.fixedRect),'source neighbor did not fill the collapsed hole');
     if(root.dragCase===28)root.dragCheck(root.area(root.windowRect(root.testB))<root.area(root.fixedRect),'occupied destination did not yield space');
     root.dragCheck(!dragPreview.visible,'preview survived release');
     console.log('TKDRAG PASS',root.dragCase);root.dragCase++;root.dragTicks=0;
     if(root.dragCase===${controlDrag?29:27}){console.log('TKDRAG DONE');this.stop();}else root.dragPhase=0;
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
                    await send('move '+data.start.x+' '+data.start.y);
                    if(controlDrag&&data.case>=27)await send('key 29 1');
                    await send('button 1');
                    await send('move '+(data.start.x+30)+' '+(data.start.y+35));
                    await send('move '+data.point.x+' '+data.point.y);
                    if(controlDrag&&data.case>=27&&!effectLoaded('tilekeep-control-marker'))throw Error('Ctrl drag did not load the marker effect');
                } else if(line.includes('TKDRAG HOVER')) {
                    if(line.includes('HOVER 18 ')){await send('key 1 1');await send('key 1 0');}
                    await send('button 0');
                    if(controlDrag&&/HOVER (27|28) /.test(line))await send('key 29 0');
                    if(controlDrag&&/HOVER (27|28) /.test(line)&&effectLoaded('tilekeep-control-marker'))throw Error('Ctrl drag marker survived Ctrl release');
                } else if(line.includes('TKDRAG DONE')) {
                    console.log(controlDrag?'PASS 28 native drag/hover/drop targets plus Escape; Ctrl free source and occupied destination verified':'PASS 26 native drag/hover/drop targets plus Escape; expanded full-space target verified');return;
                }
            }
        }
        throw Error('Native test timed out');
    } finally {
        if(input.exitCode===null){input.stdin.end('button 0\nquit\n');await wait(150);if(input.exitCode===null)input.kill('SIGTERM');}
        if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-native-drag-test');
        if(probeLoaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-resize-marker-probe');
    }
};
module.exports.prepare=prepare;
