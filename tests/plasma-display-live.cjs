// Opt-in topology replay against real KWin client frames; not a physical unplug test.
// Only the two directly spawned test windows are moved or terminated.
const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {execFileSync,spawn}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
const {capture,compare}=require('./kwin-desktop.cjs');
const dbus=(...a)=>execFileSync('qdbus6',['org.kde.KWin',...a],{encoding:'utf8'}).trim();
const wait=ms=>new Promise(r=>setTimeout(r,ms));
if(!process.argv.includes('--allow-test-windows'))throw Error('Requires --allow-test-windows');
const index=process.argv.indexOf('--area'),coords=index<0?[]:process.argv[index+1].split(',').map(Number);
if(coords.length!==4||!coords.every(Number.isInteger)||coords[2]<420||coords[3]<500)throw Error('Requires --area x,y,width,height, at least 420×500');
const area={x:coords[0],y:coords[1],width:coords[2],height:coords[3]};
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-display-live-')),marker='TKDISPLAY'+Date.now();
const fixture=path.join(dir,'windows.qml'),file=path.join(dir,'runtime.qml');let app,loaded=false;
(async()=>{
    if(dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded','tilekeep-runtime')==='true')throw Error('Stop Tilekeep first');
    const baseline=await capture();
    fs.writeFileSync(fixture,`import QtQuick
import QtQuick.Window
Window {visible:true;width:220;height:260;minimumWidth:100;minimumHeight:100;title:'${marker} A'
 Rectangle {anchors.fill:parent;color:'#243446';Text {anchors.centerIn:parent;text:'Display recovery test A';color:'white'}}
 Window {visible:true;width:220;height:260;minimumWidth:100;minimumHeight:100;title:'${marker} B';transientParent:null
 Rectangle {anchors.fill:parent;color:'#344424';Text {anchors.centerIn:parent;text:'Display recovery test B';color:'white'}}}}
`);
    app=spawn('qml6',[fixture],{stdio:'ignore'});await wait(800);if(app.exitCode!==null)throw Error('Test windows could not start');
    let source=fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8').replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_DRY_RUN__','false');
    for(const f of ['start','tileable','readDisplayState','refreshWorkAreas'])source=source.replace('function '+f+'(','function production'+f+'(');
    const test=`
 property var a: null
 property var b: null
 property var savedTree: null
 property var beforeA: null
 property var beforeB: null
 property var testArea: ${JSON.stringify(area)}
 property bool disconnected: false
 property bool panelMissing: false
 property int phase: 0
 property int ticks: 0
 property int phaseTicks: 0
 function tileable(w){return w&&w.pid===${app.pid}&&String(w.caption).startsWith('${marker}')&&productiontileable(w);}
 function readDisplayState(){return disconnected?[]:productionreadDisplayState().map(s=>({name:s.name,output:s.output,area:Object.assign({},testArea,{height:testArea.height+(panelMissing?30:0)})}));}
 function refreshWorkAreas(){return false;}
 function check(ok,msg){if(!ok)throw Error(msg);}
 function start(){
   const ws=Workspace.stackingOrder;a=ws.find(w=>String(w.caption)==='${marker} A');b=ws.find(w=>String(w.caption)==='${marker} B');
   check(a&&b,'Missing owned clients');syncMonitors();check(monitors.length===1,'Requires one real output');
   connectWindow(a);connectWindow(b);const m=monitors[0];assign(m.root,a);splitSlot(m,m.root,'y',false,b);apply();
 }
 Timer {interval:100;running:true;repeat:true;onTriggered:{try{
   if(++root.ticks>250)throw Error('Replay timeout');
   ++root.phaseTicks;
   const m=root.monitors[0];
   if(root.phase===0){if(root.currentPlacement||root.placementQueue.length)return;
     root.beforeA=root.windowRect(root.a);root.beforeB=root.windowRect(root.b);root.savedTree=m.root;
     root.disconnected=true;root.beginDisplayTransition();
     root.a.frameGeometry={x:root.testArea.x+20,y:root.testArea.y+20,width:200,height:200};
     root.b.frameGeometry={x:root.testArea.x+40,y:root.testArea.y+40,width:200,height:200};
     root.phase=1;root.phaseTicks=0;return;}
   if(root.phase===1){if(root.phaseTicks<12)return;
     root.check(root.displayTransition&&m.online===false&&m.output===null,'Missing output was not frozen');
     root.check(m.root===root.savedTree,'Offline layout was discarded');
     root.check(root.windowRect(root.a).width===200&&root.windowRect(root.b).width===200,'Unexpected placement while disconnected');
     root.check(!root.currentPlacement&&!root.placementQueue.length,'Pending writes during disconnect');
     console.log('${marker}','PASS offline cache and placement suspension on real clients');
     root.disconnected=false;root.panelMissing=true;root.phase=2;root.phaseTicks=0;return;}
   if(root.phase===2){if(root.phaseTicks<12)return;
     root.check(root.displayTransition,'Restored before work area settled');
     root.panelMissing=false;root.phase=3;root.phaseTicks=0;return;}
   if(root.phase===3){if(root.phaseTicks<12)return;
     root.check(root.displayTransition,'Late panel area did not reset debounce');
     root.check(m.root===root.savedTree&&m.area.height===root.testArea.height,'Cached area changed early');
     root.phase=4;root.phaseTicks=0;return;}
   if(root.phase===4){if(root.displayTransition||root.currentPlacement||root.placementQueue.length)return;if(root.phaseTicks<15)return;
     root.check(m.root===root.savedTree,'Reconnect replaced the tree');
     root.check(root.geometryMatches(root.windowRect(root.a),root.beforeA,1),'Client A was not restored');
     root.check(root.geometryMatches(root.windowRect(root.b),root.beforeB,1),'Client B was not restored');
     root.check(root.allWindows(m.root).length===2,'Membership duplicated');
     console.log('${marker}','PASS stable panel recovery and exact client restoration');console.log('${marker}','DONE');this.stop();
   }
 }catch(e){console.log('${marker}','FAIL',String(e));this.stop();}}}
 `;
    fs.writeFileSync(file,source.slice(0,source.lastIndexOf('}'))+test+'}\n');
    loadScript(dbus,file,marker);loaded=true;
    const seen=new Set();
    for(let i=0;i<100;i++) {
        await wait(300);
        const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','--since','-2 minutes','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
        const lines=log.split('\n').filter(l=>l.includes(marker));
        for(const l of lines)if(!seen.has(l)){console.log(l);seen.add(l);}
        if(lines.some(l=>l.includes('FAIL')))throw Error('Live display replay failed');
        if(lines.some(l=>l.includes('DONE'))){console.log('PASS preserved',compare(baseline,await capture()),'preexisting user windows');return;}
    }
    throw Error('No replay completion received');
})().catch(e=>{console.error(e.message);process.exitCode=1;}).finally(async()=>{
    if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',marker);
    if(app&&app.exitCode===null){app.kill('SIGTERM');await wait(300);}
    for(const f of [file,fixture])if(fs.existsSync(f))fs.unlinkSync(f);fs.rmdirSync(dir);
});
