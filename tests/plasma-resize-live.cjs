// Opt-in compositor/input test. Manages ONLY three windows spawned by this test.
// Requires qml6 and Python evdev with /dev/uinput access. No global close action.
const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {execFileSync,spawn}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
if(!process.argv.includes('--allow-test-windows'))throw Error('Requires --allow-test-windows');
if(!process.argv.includes('--allow-live-compositor-risk'))throw Error('Live KWin tests can crash the desktop session. Prefer plasma-isolated.cjs; --allow-live-compositor-risk is required on a disposable desktop.');
const areaIndex=process.argv.indexOf('--area');
const coordinates=areaIndex>=0?process.argv[areaIndex+1].split(',').map(Number):[];
if(coordinates.length!==4||!coordinates.every(Number.isInteger)||coordinates[2]<420||coordinates[3]<500)throw Error('Requires --area x,y,width,height (at least 420×500) on the active display');
const testArea={x:coordinates[0],y:coordinates[1],width:coordinates[2],height:coordinates[3]};
const dbus=(...a)=>execFileSync('qdbus6',['org.kde.KWin',...a],{encoding:'utf8'}).trim();
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-resize-live-')),marker='TKRESIZE'+Date.now();
const fixture=path.join(dir,'windows.qml'),script=path.join(dir,'runtime.qml');
fs.writeFileSync(fixture,`import QtQuick
import QtQuick.Window
Window { visible:true; width:240; height:300; minimumWidth:100; minimumHeight:100; title:"${marker} A"
 Rectangle {anchors.fill:parent;color:"#243446"; Text {anchors.centerIn:parent;text:"Tilekeep resize test A";color:"white"}}
 Window {visible:true; width:240;height:300;minimumWidth:100;minimumHeight:100;title:"${marker} B";transientParent:null
 Rectangle {anchors.fill:parent;color:"#344424"; Text {anchors.centerIn:parent;text:"Connected window below B";color:"white"}}}
 Window {visible:true; width:240;height:300;minimumWidth:100;minimumHeight:100;title:"${marker} C";transientParent:null
 Rectangle {anchors.fill:parent;color:"#443424"; Text {anchors.centerIn:parent;text:"Across the edge C";color:"white"}}}
}`);
const wait=ms=>new Promise(r=>setTimeout(r,ms));
let app,loaded=false;
(async()=>{
    if(dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded','tilekeep-runtime')==='true')throw Error('Stop Tilekeep first; this test never stops a user runtime itself.');
    app=spawn('qml6',[fixture],{stdio:['ignore','ignore','pipe']});let appErrors='';app.stderr.on('data',b=>appErrors+=b);
    await wait(700);if(app.exitCode!==null)throw Error(appErrors);
    let source=fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8').replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_FLOAT_SECONDARY_WINDOWS__','true').replace('__TILEKEEP_DRY_RUN__','false');
    source=source.replace('function tileable(w)','function productionTileable(w)').replace('function start()','function productionStart()').replace('function refreshWorkAreas()','function productionRefreshWorkAreas()');
    source=source.replace('function displaysReady()','function productionDisplaysReady()').replace('function pollDisplays()','function productionPollDisplays()');
    source=source.replace('drag.raw=raw;',`console.log('${marker}','STEP',JSON.stringify({g,raw,before:drag.rect,cursor:Workspace.cursorPos,start:drag.cursor,edges:drag.edges}));drag.raw=raw;`)
        .replace('const final=windowRect(w);',`const final=windowRect(w);console.log('${marker}','FINISH',JSON.stringify({final,before:d.rect,raw:d.raw,requested:w.moveResizeGeometry}));`);
    const test=`
 property var a: null
 property var b: null
 property var c: null
 property var base: null
 property var fixed: null
 property var across: null
 property var wanted: null
 property var userWindows: []
 property int phase: 0
 property int ticks: 0
 property int nativeTicks: 0
 property bool sawNativeSnap: false
 property bool sawNativeFollow: false
 property var guideFocus: null
 property var cancelTree: null
 function tileable(w){return w&&w.pid===${app.pid}&&String(w.caption).startsWith('${marker}')&&productionTileable(w);}
 function refreshWorkAreas(){return false;}
 function displaysReady(){return true;}
 function pollDisplays(){return false;}
 function check(ok,msg){if(!ok)throw Error(msg);}
 function verifyUsers(){for(const p of userWindows)if(!p.w.deleted)check(geometryMatches(windowRect(p.w),p.rect),'User window moved: '+identity(p.w));}
 function verifyConnected(){
   const ra=windowRect(a),rb=windowRect(b),rc=windowRect(c);
   check(geometryMatches(rb,Object.assign({},fixed,{width:ra.width}),1),'window below lost shared edge');
   check(Math.abs(rc.x-rectRight(ra)-gap)<=1,'window across lost shared edge');
   check(Math.abs(rectRight(rc)-rectRight(across))<=1&&Math.abs(rc.y-across.y)<=1&&Math.abs(rc.height-across.height)<=1,'across window outer edges moved');
 }
 function start(){
   const ws=Workspace.stackingOrder;
   userWindows=ws.filter(w=>w.normalWindow&&w.pid!==${app.pid}).map(w=>({w,rect:windowRect(w)}));
   a=ws.find(w=>String(w.caption)==='${marker} A');b=ws.find(w=>String(w.caption)==='${marker} B');c=ws.find(w=>String(w.caption)==='${marker} C');
   check(a&&b&&c,'test windows missing');connectWindow(a);connectWindow(b);connectWindow(c);
   const left=leaf(),empty=leaf(),upper=leaf(),lower=leaf();assign(upper,a);assign(lower,b);
   const row={kind:'split',axis:'y',ratio:.5,first:upper,second:lower,parent:null};upper.parent=row;lower.parent=row;
   const tree={kind:'split',axis:'x',ratio:.5,first:row,second:empty,parent:null};row.parent=tree;empty.parent=tree;
   assign(empty,c);
   // A small controlled region; the rest of the desktop is excluded entirely.
   monitors=[{name:'resize-test',output:a.output,area:${JSON.stringify(testArea)},root:tree}];
   focused=a;apply();
 }
 DBusCall {id: nativeResize;service:'org.kde.kglobalaccel';path:'/component/kwin';dbusInterface:'org.kde.kglobalaccel.Component';method:'invokeShortcut';arguments:['Window Resize']}
 Timer {interval:150;running:true;repeat:true;onTriggered:{try{
   if(++root.ticks>180)throw Error('test timeout');
   if(root.currentPlacement||root.placementQueue.length)return;
   const m=root.monitors[0];
   if(root.phase===0){root.base=root.windowRect(root.a);root.fixed=root.windowRect(root.b);root.across=root.windowRect(root.c);root.wanted=Object.assign({},root.base,{width:root.base.width+110});root.check(root.adjustRatio(root.a,root.base,root.wanted),'local resize rejected');root.apply();root.phase=1;return;}
   if(root.phase===1){root.check(root.geometryMatches(root.windowRect(root.a),root.wanted),'resized frame mismatch');root.verifyConnected();root.verifyUsers();console.log('${marker}','PASS connected resize preserves unrelated windows');
     const bounds=root.inset(m.area),raw=Object.assign({},root.wanted,{width:Math.round(bounds.width*.5)+4});
     const snap=root.resizeSnap(root.a,root.wanted,raw,['right']);root.check(snap.guides.length===1,'snap guide missing '+JSON.stringify({raw,snap,min:root.minimumSize(root.slotOf(root.c)[1]),plan:root.resizeLayout(root.a,root.wanted,raw)?.rect}));root.guideFocus=Workspace.activeWindow;root.showResizeGuide(root.a,snap);root.phase=2;return;}
   if(root.phase===2){root.check(resizeGuide.visible,'native snap guide not visible');root.check(Workspace.activeWindow===root.guideFocus,'guide stole focus');root.hidePreview();console.log('${marker}','PASS real snap guide surface');
     const bounds=root.inset(m.area);root.base=Object.assign({},root.base,{width:Math.round(bounds.width*.5)-20});
     root.adjustRatio(root.a,root.wanted,root.base);root.apply();root.phase=3;return;}
   if(root.phase===3){root.check(root.geometryMatches(root.windowRect(root.a),root.base),'reset test geometry');Workspace.activeWindow=root.a;root.check(Workspace.activeWindow===root.a,'test focus');nativeResize.call();root.phase=4;return;}
   if(root.phase===4){if(root.a.resize&&Workspace.activeWindow===root.a){console.log('${marker}','INPUT_READY');root.phase=5;}return;}
   if(root.phase===5||root.phase===8||root.phase===11){if(resizeGuide.visible)root.sawNativeSnap=true;
     if(root.a.resize||root.interactiveWindows.size){if(root.windowRect(root.a).width!==root.base.width&&Math.abs(root.windowRect(root.b).width-root.windowRect(root.a).width)<=1)root.sawNativeFollow=true;return;}
     if(++root.nativeTicks<4)return;
     root.check(root.sawNativeSnap,'native input never captured a snap');root.check(root.sawNativeFollow,'connected neighbor did not follow during native drag');
     if(root.phase===11){root.check(root.geometryMatches(root.windowRect(root.a),root.base),'Escape did not restore active window');root.check(m.root===root.cancelTree,'Escape changed saved tree');root.verifyConnected();root.verifyUsers();root.check(!resizeGuide.visible&&!root.resizePreviewWindows.size,'Escape left a preview');console.log('${marker}','PASS native Escape restores connected neighbors');console.log('${marker}','DONE');root.phase=12;return;}
     const halfway=Math.round(root.inset(m.area).width*.5);
     root.check(root.phase===5?root.windowRect(root.a).width>halfway+6:root.windowRect(root.a).width<halfway-6,'native resize did not escape the snap '+JSON.stringify({phase:root.phase,actual:root.windowRect(root.a),halfway,area:m.area}));
     root.check(root.geometryMatches(root.windowRect(root.a),root.rects(m).get(root.slotOf(root.a)[1])),'native release snapped back');root.verifyConnected();root.verifyUsers();root.check(!resizeGuide.visible,'guide remains after release');
     if(root.phase===5){console.log('${marker}','PASS native growing edge, live neighbor follow, snap and release');root.phase=6;return;}
     console.log('${marker}','PASS native shrinking edge, live neighbor follow, snap and release');root.phase=9;return;
   }
   if(root.phase===6){root.base=root.windowRect(root.a);root.nativeTicks=0;root.sawNativeSnap=false;root.sawNativeFollow=false;Workspace.activeWindow=root.a;nativeResize.call();root.phase=7;return;}
   if(root.phase===7){if(root.a.resize&&Workspace.activeWindow===root.a){console.log('${marker}','SHRINK_READY');root.phase=8;}return;}
   if(root.phase===9){root.base=root.windowRect(root.a);root.cancelTree=m.root;root.nativeTicks=0;root.sawNativeSnap=false;root.sawNativeFollow=false;Workspace.activeWindow=root.a;nativeResize.call();root.phase=10;return;}
   if(root.phase===10){if(root.a.resize&&Workspace.activeWindow===root.a){console.log('${marker}','CANCEL_READY');root.phase=11;}return;}
 }catch(e){root.hidePreview();console.log('${marker}','FAIL',String(e));root.phase=6;this.stop();}}}
 `;
    fs.writeFileSync(script,source.slice(0,source.lastIndexOf('}'))+test+'}\n');
    loadScript(dbus,script,marker);loaded=true;
    let sent=false,sentShrink=false,sentCancel=false,seen=new Set();
    for(let i=0;i<100;i++){
        await wait(300);
        const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','--since','-2 minutes','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
        const lines=log.split('\n').filter(s=>s.includes(marker));for(const line of lines)if(!seen.has(line)){console.log(line);seen.add(line);}
        if(lines.some(l=>l.includes('FAIL')))throw Error('Live resize checks failed');
        const grow=!sent&&lines.some(l=>l.includes('INPUT_READY')),shrink=!sentShrink&&lines.some(l=>l.includes('SHRINK_READY'));
        const cancel=!sentCancel&&lines.some(l=>l.includes('CANCEL_READY'));
        if(grow||shrink||cancel){if(grow)sent=true;else if(shrink)sentShrink=true;else sentCancel=true;
            // Only directional resize input and Enter; never Alt-F4 or Window Close.
            execFileSync('/usr/bin/python',['-c',`from evdev import UInput, ecodes as e
import time
with UInput({e.EV_KEY:[e.KEY_RIGHT,e.KEY_LEFT,e.KEY_ENTER,e.KEY_ESC]},name='Tilekeep resize verification keyboard') as ui:
 time.sleep(0.6)
 for key in [${shrink?"e.KEY_LEFT":"e.KEY_RIGHT"}]*5+[${cancel?"e.KEY_ESC":"e.KEY_ENTER"}]:
  ui.write(e.EV_KEY,key,1);ui.syn();time.sleep(0.12);ui.write(e.EV_KEY,key,0);ui.syn();time.sleep(0.12)
`],{timeout:6000});
        }
        if(lines.some(l=>l.includes('DONE')))return;
    }
    throw Error('Live test timed out');
})().catch(e=>{console.error(e.message);process.exitCode=1;}).finally(async()=>{
    if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',marker);
    // These three disposable windows belong to the child this test created.
    if(app&&app.exitCode===null){app.kill('SIGTERM');await wait(300);}
    for(const f of [fixture,script])if(fs.existsSync(f))fs.unlinkSync(f);fs.rmdirSync(dir);
});
