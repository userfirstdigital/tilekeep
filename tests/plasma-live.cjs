// Opt-in integration test of the real QML backend on the current Plasma desktop.
// Rearranges existing windows; never closes, minimizes, or launches an app window.
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {execFileSync}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
if(!process.argv.includes('--allow-window-moves'))throw Error('Use --allow-window-moves on a disposable or explicitly authorized desktop.');
const dbus=(...args)=>execFileSync('qdbus6',['org.kde.KWin',...args],{encoding:'utf8'}).trim();
const plugin='tilekeep-runtime';
const runId=`TKLIVE-${Date.now()}`;
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-live-'));
const fixture=path.join(dir,'test.qml');
const source=fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8')
    .replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_DRY_RUN__','false');
const testQml=`
    property int testStep: 0
    property int settling: 0
    property var testWindows: []
    property var testRoots: []
    property var testFocus: null
    property var testStripWindow: null
    property var testStripRect: null
    function cloneTree(n,parent) {
        const copy=Object.assign({},n,{parent});
        if(n.kind==="leaf")copy.windows=n.windows.slice();
        else {copy.first=cloneTree(n.first,copy);copy.second=cloneTree(n.second,copy);}
        return copy;
    }
    function check(ok,message) { if(!ok)throw Error(message); }
    function verifyGeometry() {
        const visible=placements().filter(p=>visibleHere(p.window));
        for(const p of visible)check(geometryMatches(windowRect(p.window),p.rect),identity(p.window)+" geometry mismatch: "+JSON.stringify(windowRect(p.window))+" wanted "+JSON.stringify(p.rect));
        check(visible.length>=2,"Need at least two visible windows");
    }
    function shortcut(name) { testShortcut.arguments=[name];testShortcut.call(); }
    function logPass(name) { console.log("${runId}","PASS",name); }
    Timer {
        interval: 250; running: true; repeat: true
        onTriggered: {
            if(root.currentPlacement||root.placementQueue.length) {
                if(++root.settling<160)return;
                console.log("${runId}","FAIL","placement did not settle");stop();return;
            }
            try {
                switch(root.testStep++) {
                case 0:
                    root.verifyGeometry();root.logPass("initial tiling");
                    root.testFocus=Workspace.activeWindow;
                    root.testRoots=root.monitors.map(m=>root.cloneTree(m.root,null));
                    root.testWindows=root.placements().map(p=>p.window).filter(w=>root.visibleHere(w));
                    // Use two ordinary KDE/browser windows with modest minimum sizes.
                    root.testWindows.sort((a,b)=>Number(root.identity(b)==="org.kde.dolphin")-Number(root.identity(a)==="org.kde.dolphin"));
                    Workspace.activeWindow=root.testWindows[0];
                    root.shortcut("TilekeepFloat");break;
                case 1:
                    root.check(root.floating.has(root.testWindows[0]),"float shortcut");root.logPass("float shortcut");
                    root.shortcut("TilekeepFloat");break;
                case 2:
                    root.check(!root.floating.has(root.testWindows[0]),"unfloat shortcut");root.verifyGeometry();root.logPass("unfloat shortcut");
                    root.shortcut("TilekeepRetile");break;
                case 3: {
                    root.verifyGeometry();root.logPass("retile shortcut");
                    const w=root.testWindows[0], target=root.slotOf(root.testWindows[1]);
                    const r=root.rects(target[0]).get(target[1]);
                    root.drop(w,{x:r.x+r.width/2,y:r.y+r.height/2},false);root.apply();break;
                }
                case 4: {
                    root.verifyGeometry();root.logPass("center-drop swap");
                    const w=root.testWindows[0],target=root.slotOf(root.testWindows[1]);
                    const r=root.rects(target[0]).get(target[1]);
                    root.drop(w,{x:r.x+r.width/2,y:r.y+1},false);root.apply();break;
                }
                case 5: {
                    root.verifyGeometry();root.logPass("edge-drop split");
                    const w=root.testWindows[0],before=root.windowRect(w);
                    root.adjustRatio(w,before,Object.assign({},before,{height:before.height+60}));root.apply();break;
                }
                case 6: {
                    root.verifyGeometry();root.logPass("nested resize");
                    const w=root.testWindows[0],target=root.slotOf(root.testWindows[1]);
                    const r=root.rects(target[0]).get(target[1]);
                    root.drop(w,{x:r.x+r.width/2,y:r.y+r.height/2},true);root.apply();break;
                }
                case 7:
                    root.verifyGeometry();root.check(root.slotOf(root.testWindows[0])[1]===root.slotOf(root.testWindows[1])[1],"stack");root.logPass("stack");
                    root.shortcut("TilekeepNext");break;
                case 8:
                    root.check(Workspace.activeWindow===root.testWindows[1],"next stack shortcut focus");root.logPass("next stack shortcut");
                    root.shortcut("TilekeepPrevious");break;
                case 9:
                    root.check(Workspace.activeWindow===root.testWindows[0],"previous stack shortcut focus");root.logPass("previous stack shortcut");
                    root.shortcut("TilekeepCompact");break;
                case 10: {
                    root.verifyGeometry();root.logPass("compact shortcut");
                    const w=root.testWindows[0],found=root.slotOf(w),target=found[1];
                    root.testStripWindow=w;root.testStripRect=root.rects(found[0]).get(target);
                    const vacancy=root.leaf();
                    const split={kind:"split",axis:"y",ratio:.95,first:target,second:vacancy,parent:null};
                    root.replaceNode(target,split,found[0]);target.parent=split;vacancy.parent=split;
                    root.apply();break;
                }
                case 11:
                    root.verifyGeometry();
                    root.check(root.geometryMatches(root.windowRect(root.testStripWindow),root.testStripRect),"vacant strip and its gap were not absorbed");
                    root.logPass("collapsed vacancy leaves no extra strip");
                    for(let i=0;i<root.monitors.length;i++)root.monitors[i].root=root.testRoots[i];
                    root.floating.clear();root.apply();break;
                case 12:
                    root.verifyGeometry();Workspace.activeWindow=root.testFocus;
                    root.logPass("restored initial layout");console.log("${runId}","DONE");stop();break;
                }
                root.settling=0;
            } catch(e) {
                console.log("${runId}","FAIL",String(e));
                if(root.testRoots.length) {for(let i=0;i<root.monitors.length;i++)root.monitors[i].root=root.testRoots[i];root.floating.clear();root.apply();}
                stop();
            }
        }
    }
    DBusCall { id: testShortcut; service:"org.kde.kglobalaccel"; path:"/component/kwin"; method:"invokeShortcut" }
`;
fs.writeFileSync(fixture,source.slice(0,source.lastIndexOf('}'))+testQml+'}\n');
(async()=>{
    try {
        // An active app instance owns this plugin; let it observe unloading first.
        dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',plugin);
        await new Promise(r=>setTimeout(r,1000));
        loadScript(dbus,fixture,plugin);
        let printed='';
        for(let i=0;i<90;i++) {
            await new Promise(r=>setTimeout(r,1000));
            const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','-n','3000','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
            const lines=log.split('\n').filter(l=>l.includes(runId)).join('\n');
            if(lines!==printed){console.log(lines.slice(printed.length).trim());printed=lines;}
            if(lines.includes('FAIL'))throw Error('Live test failed');
            if(lines.includes('DONE'))return;
        }
        throw Error('Timed out waiting for live test');
    } finally {
        dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',plugin);
        fs.unlinkSync(fixture);fs.rmdirSync(dir);
    }
})().catch(e=>{console.error(e.message);process.exitCode=1;});
