// Opt-in real Plasma geometry/overlay checks. Existing windows stay open.
const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {execFileSync}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
const args=process.argv.slice(2),snapshotIndex=args.indexOf('--snapshot');
if(!args.includes('--allow-window-moves')||snapshotIndex<0)throw Error('Requires --allow-window-moves --snapshot PATH');
const snapshot=JSON.parse(fs.readFileSync(args[snapshotIndex+1],'utf8'));
const screenshotIndex=args.indexOf('--screenshot');
const dbus=(...a)=>execFileSync('qdbus6',['org.kde.KWin',...a],{encoding:'utf8'}).trim();
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-zones-')),file=path.join(dir,'zones.qml'),id='TKZONES'+Date.now();
const source=fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8').replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_FLOAT_SECONDARY_WINDOWS__','true').replace('__TILEKEEP_DRY_RUN__','false');
const qml=`
    property int phase: 0
    property int zoneIndex: 0
    property int hold: 0
    property int settle: 0
    property int previewShows: 0
    property int previewHides: 0
    property var testWindow: null
    property var testMonitor: null
    property var testMembers: []
    property var testPoint: null
    property var testBounds: null
    property var testExpected: null
    property var savedTestSnapshot: ${JSON.stringify(JSON.stringify(snapshot))}
    function check(ok,message) {if(!ok)throw Error(message);}
    Connections {target:dragPreview;function onVisibleChanged(){if(dragPreview.visible)root.previewShows++;else root.previewHides++;}}
    Timer {interval:250;running:true;repeat:true;onTriggered:{
        try {
            if(root.currentPlacement||root.placementQueue.length){if(++root.settle>160)throw Error('placements failed to settle');return;}root.settle=0;
            if(root.phase===0){root.restoreSnapshot(root.savedTestSnapshot);root.phase=1;return;}
            if(root.phase===1){
                root.testWindow=Workspace.stackingOrder.find(w=>root.tileable(w)&&root.visibleHere(w)&&root.identity(w)==='org.kde.dolphin');
                root.check(!!root.testWindow,'Need a visible Dolphin window');
                root.testMonitor=root.slotOf(root.testWindow)[0];root.testMembers=root.allWindows(root.testMonitor.root);root.phase=2;
            }
            if(root.phase===2){
                const occupied=root.leaf(),empty=root.leaf();for(const w of root.testMembers)root.assign(occupied,w);
                const split={kind:'split',axis:'x',ratio:.5,first:occupied,second:empty,parent:null};occupied.parent=split;empty.parent=split;
                root.testMonitor.root=split;root.apply();root.phase=3;return;
            }
            if(root.phase===3){
                Workspace.activeWindow=root.testWindow;
                const target=root.testMonitor.root.second,r=root.rects(root.testMonitor).get(target);
                const xs=[.1,.5,.9],x=xs[root.zoneIndex%3],y=xs[Math.floor(root.zoneIndex/3)];
                root.testPoint={x:r.x+r.width*x,y:r.y+r.height*y};root.testBounds=r;
                const z=root.fittingEmptyZone(root.testWindow,r,root.emptyZone(r,root.testPoint));
                root.testExpected=root.dropPreview(root.testWindow,[root.testMonitor,target,r],z);
                root.showPreview(root.testWindow,root.testExpected,r,z);root.hold=0;root.phase=4;
                console.log('${id}','PREVIEW',z);return;
            }
            if(root.phase===4){
                root.check(dragPreview.visible,'preview flickered off');
                root.check(Workspace.activeWindow===root.testWindow,'preview stole focus');
                root.check(root.previewShows===root.zoneIndex+1&&root.previewHides===root.zoneIndex,'preview changed visibility during hover');
                // Repeated hover updates retain the same full-area guide surface.
                for(let i=0;i<30;i++)root.showPreview(root.testWindow,root.testExpected,root.testBounds,root.previewZone);
                if(++root.hold<4)return;
                root.hidePreview();root.drop(root.testWindow,root.testPoint,false);root.apply();root.phase=5;return;
            }
            if(root.phase===5){
                root.check(root.geometryMatches(root.windowRect(root.testWindow),root.testExpected),'drop geometry differs from preview');
                root.check(root.testMembers.every(w=>root.slotOf(w)),'an existing window was lost');
                console.log('${id}','PASS','zone',root.zoneIndex,'preview, focus and final geometry');
                if(++root.zoneIndex<9){root.phase=2;return;}
                root.restoreSnapshot(root.savedTestSnapshot);root.phase=6;return;
            }
            if(root.phase===6){
                for(const p of root.placements())if(root.visibleHere(p.window))root.check(root.geometryMatches(root.windowRect(p.window),p.rect),'restored geometry mismatch');
                console.log('${id}','PASS','restored original snapshot');console.log('${id}','DONE');stop();
            }
        }catch(e){root.hidePreview();root.restoreSnapshot(root.savedTestSnapshot);console.log('${id}','FAIL',String(e));stop();}
    }}
`;
fs.writeFileSync(file,source.slice(0,source.lastIndexOf('}'))+qml+'}\n');
(async()=>{
    let captured=false;
    try {
        dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-runtime');
        await new Promise(r=>setTimeout(r,1000));loadScript(dbus,file,'tilekeep-runtime');
        let printed='';
        for(let i=0;i<120;i++){
            await new Promise(r=>setTimeout(r,500));
            const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','-n','2000','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
            const lines=log.split('\n').filter(l=>l.includes(id)).join('\n');
            if(lines!==printed){console.log(lines.slice(printed.length).trim());printed=lines;}
            if(!captured&&screenshotIndex>=0&&lines.includes('PREVIEW')){
                execFileSync('spectacle',['--background','--nonotify','--fullscreen','--output',args[screenshotIndex+1]],{timeout:10000});captured=true;
            }
            if(lines.includes('FAIL'))throw Error('Live zone verification failed');
            if(lines.includes('DONE'))return;
        }
        throw Error('Live zone verification timed out');
    }finally{dbus('/Scripting','org.kde.kwin.Scripting.unloadScript','tilekeep-runtime');fs.unlinkSync(file);fs.rmdirSync(dir);}
})().catch(e=>{console.error(e.message);process.exitCode=1;});
