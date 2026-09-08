// A private headless compositor, private D-Bus session, and software rendering.
// Never uses the user's KWin service, display socket, or /dev/uinput.
const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {spawn,execFileSync}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const rootDir=process.env.TILEKEEP_ISOLATED_DIR;
if(!process.argv.includes('--inside')) {
    const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-isolated-'));
    for(const sub of ['runtime','config','cache','data'])fs.mkdirSync(path.join(dir,sub),{mode:0o700});
    const env={...process.env,TILEKEEP_ISOLATED_DIR:dir,XDG_RUNTIME_DIR:path.join(dir,'runtime'),XDG_CONFIG_HOME:path.join(dir,'config'),XDG_CACHE_HOME:path.join(dir,'cache'),XDG_DATA_HOME:path.join(dir,'data'),QT_QUICK_BACKEND:'software',QT_FORCE_STDERR_LOGGING:'1',QT_QPA_PLATFORMTHEME:'generic',LIBGL_ALWAYS_SOFTWARE:'1',WAYLAND_DISPLAY:'tilekeep-isolated'};
    for(const name of ['DBUS_SESSION_BUS_ADDRESS','DISPLAY','XAUTHORITY','SESSION_MANAGER','WAYLAND_SOCKET','JOURNAL_STREAM','APPIMAGE','APPDIR'])delete env[name];
    const runnerLog=fs.openSync(path.join(dir,'runner.log'),'wx',0o600);
    const child=spawn('dbus-run-session',['--',process.execPath,__filename,'--inside',...process.argv.slice(2)],{env,stdio:['ignore','inherit',runnerLog]});
    child.on('exit',code=>{fs.closeSync(runnerLog);console.log('Isolated diagnostics:',dir);process.exitCode=code??1;});
} else (async()=>{
    if(!rootDir||!rootDir.startsWith(os.tmpdir()+'/tilekeep-isolated-')||process.env.XDG_RUNTIME_DIR!==path.join(rootDir,'runtime'))throw Error('Missing isolated runtime');
    const logFile=path.join(rootDir,'kwin.log'),fd=fs.openSync(logFile,'wx',0o600);
    // Qt caches directory entries when resolving local QML component names.
    for(let i=0;i<40;i++)fs.writeFileSync(path.join(rootDir,'TKISOLATED'+i+'.qml'),'');
    const env={...process.env,QT_QPA_PLATFORM:'offscreen'};
    const mesa='/usr/share/glvnd/egl_vendor.d/50_mesa.json';if(fs.existsSync(mesa))env.__EGL_VENDOR_LIBRARY_FILENAMES=mesa;
    if(process.argv.includes('--native-drag'))require('./plasma-drag-isolated.cjs').prepare(rootDir,fd);
    const scale=process.argv.includes('--fractional-scale')?'1.25':'1';
    const kwin=spawn('kwin_wayland',['--virtual','--width','1200','--height','800','--scale',scale,'--no-lockscreen','--no-kactivities','--socket','tilekeep-isolated'],{env,stdio:['ignore',fd,fd]});
    const dbus=(...a)=>execFileSync('qdbus6',['org.kde.KWin',...a],{encoding:'utf8',timeout:3000,stdio:['ignore','pipe','pipe']}).trim();
    let app;
    try {
        let ready=false;
        for(let i=0;i<100;i++){if(kwin.exitCode!==null)throw Error('Private KWin exited during startup');try{dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded','tilekeep-runtime');ready=true;break;}catch{}await wait(100);}
        if(!ready)throw Error('Private KWin did not start');
        if(process.argv.includes('--fractional-scale')) {
            const screenEnv={...process.env,QT_QPA_PLATFORM:'wayland'};
            const config=JSON.parse(execFileSync('kscreen-doctor',['-j'],{env:screenEnv,encoding:'utf8',stdio:['ignore','pipe',fd]}));
            const output=config.outputs.find(o=>o.enabled&&o.connected);
            if(!output)throw Error('No private output to scale');
            execFileSync('kscreen-doctor',[`output.${output.name}.scale.1.25`],{env:screenEnv,stdio:['ignore',fd,fd]});
            const actual=JSON.parse(execFileSync('kscreen-doctor',['-j'],{env:screenEnv,encoding:'utf8',stdio:['ignore','pipe',fd]})).outputs.find(o=>o.name===output.name);
            if(actual?.scale!==1.25)throw Error('Private output did not adopt 125% scaling');
            console.log('Verified private output scale:',actual.scale);
        }
        const fixture=path.join(rootDir,'clients.qml');
        fs.writeFileSync(fixture,`import QtQuick
import QtQuick.Window
Window {visible:true;width:300;height:300;minimumWidth:100;minimumHeight:100;title:'Tilekeep isolated A'
 Window {visible:true;width:300;height:300;minimumWidth:100;minimumHeight:100;title:'Tilekeep isolated B';transientParent:null}}
`);
        app=spawn('qml6',[fixture],{env:{...process.env,QT_QPA_PLATFORM:'wayland'},stdio:['ignore',fd,fd]});
        await wait(500);
        if(process.argv.includes('--native-drag')) {
            await require('./plasma-drag-isolated.cjs')({rootDir,kwin,app,dbus,logFile,fd,baseline:process.argv.includes('--baseline')});
            return;
        }
        const original=fs.readFileSync(path.join(__dirname,'../src/linux/kwin.qml'),'utf8').replace('__TILEKEEP_GAP__','1').replace('__TILEKEEP_DRY_RUN__','false');
        for(let cycle=0;cycle<40;cycle++) {
            if(kwin.exitCode!==null||app.exitCode!==null)throw Error('Isolated compositor or clients exited');
            const marker='TKISOLATED'+cycle,file=path.join(rootDir,marker+'.qml');
            const test=`
 property int testTicks: 0
 Timer {interval:80;running:true;repeat:true;onTriggered:{try{
   if(++root.testTicks>60)throw Error('placement timeout');
   if(root.currentPlacement||root.placementQueue.length)return;
   const w=Workspace.stackingOrder.find(w=>String(w.caption)==='Tilekeep isolated A');
   if(!w||!root.slotOf(w))throw Error('owned client missing');
   const r=root.windowRect(w),h=root.windowConnections.get(w);
   root.showPreview(w,r,null,'center');root.hidePreview();
   root.showResizeGuide(w,{guides:[{edge:'right',pos:root.rectRight(r),label:'Test'}]});
   h.started();h.stepped(Object.assign({},r,{width:r.width-30}));h.finished();
   ${cycle%2===0?'root.quiesce();':''}
   console.log('${marker}','READY');this.stop();
 }catch(e){console.log('${marker}','FAIL',String(e));this.stop();}}}
 `;
            fs.writeFileSync(file,original.slice(0,original.lastIndexOf('}'))+test+'}\n');
            loadScript(dbus,file,marker);
            let done=false;
            for(let i=0;i<70;i++) {
                await wait(100);if(kwin.exitCode!==null)throw Error('Private KWin crashed in cycle '+cycle);
                const log=fs.readFileSync(logFile,'utf8');
                if(log.includes(marker+' FAIL')||log.includes('Component failed to load'))throw Error('Fixture failed in cycle '+cycle);
                if(log.includes(marker+' READY')){done=true;break;}
            }
            if(!done)throw Error('Fixture timed out in cycle '+cycle);
            // Alternate explicit quiescence and direct unload with pending work.
            dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',marker);
            await wait(150);
            if(kwin.exitCode!==null)throw Error('Private KWin crashed on teardown '+cycle);
        }
        const log=fs.readFileSync(logFile,'utf8');
        if(/ReferenceError:|TypeError:|Cannot read property|QQmlComponent: Component is not ready/.test(log))throw Error('QML error in isolated run; inspect local log');
        console.log('PASS 40 isolated load/resize/overlay/unload cycles; private compositor and clients survived');
    } finally {
        if(app&&app.exitCode===null)app.kill('SIGTERM');
        if(kwin.exitCode===null)kwin.kill('SIGTERM');
        fs.closeSync(fd);
    }
})().catch(e=>{console.error(e.message);process.exitCode=1;});
