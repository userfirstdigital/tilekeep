// Read-only compositor probe. Captures stay local: never publish user window data.
const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
const {execFileSync}=require('node:child_process');
const {loadScript}=require('./kwin-loader.cjs');
const dbus=(...a)=>execFileSync('qdbus6',['org.kde.KWin',...a],{encoding:'utf8'}).trim();
async function capture() {
    const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-desktop-'));
    const file=path.join(dir,'probe.qml'),marker='TKDESKTOP'+Date.now();let loaded=false;
    fs.writeFileSync(file,`import QtQuick
import org.kde.kwin 3.0
QtObject {Component.onCompleted:{
 const rect=r=>({x:r.x,y:r.y,width:r.width,height:r.height});
 console.log('${marker}',JSON.stringify({
 screens:Workspace.screens.map(o=>({name:o.name,area:rect(Workspace.clientArea(KWin.MaximizeArea,o,Workspace.currentDesktop))})),
 windows:Workspace.stackingOrder.filter(w=>w.normalWindow).map(w=>({id:String(w.internalId),app:String(w.desktopFileName||w.resourceClass||''),pid:w.pid,title:String(w.caption),minimized:w.minimized,hidden:w.hidden,fullScreen:w.fullScreen,maximizeMode:Number(w.maximizeMode||0),rect:rect(w.frameGeometry)}))
 }));
}}`);
    try {
        loadScript(dbus,file,marker);loaded=true;
        for(let i=0;i<20;i++) {
            await new Promise(r=>setTimeout(r,100));
            const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','--since','-1 minute','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
            const line=log.split('\n').find(l=>l.includes(marker+' {'));
            if(line)return JSON.parse(line.slice(line.indexOf(marker)+marker.length+1));
        }
        throw Error('Compositor probe did not return');
    }finally {
        if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',marker);
        fs.unlinkSync(file);fs.rmdirSync(dir);
    }
}
async function restore(saved) {
    const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tilekeep-desktop-'));
    const file=path.join(dir,'restore.qml'),marker='TKRESTORE'+Date.now();let loaded=false;
    fs.writeFileSync(file,`import QtQuick
import org.kde.kwin 3.0
QtObject {Component.onCompleted:{
 const saved=${JSON.stringify(saved)};let restored=0,missing=0;
 for(const item of saved.windows) {
  const w=Workspace.stackingOrder.find(w=>String(w.internalId)===item.id);
  if(!w){missing++;continue;}
  if(item.minimized||item.hidden||item.fullScreen||item.maximizeMode)continue;
  if(w.tile)w.tile.unmanage(w);w.setMaximize(false,false);w.frameGeometry=item.rect;restored++;
 }
 console.log('${marker}',JSON.stringify({restored,missing}));
}}`);
    try {
        loadScript(dbus,file,marker);loaded=true;
        for(let i=0;i<20;i++) {
            await new Promise(r=>setTimeout(r,100));
            const log=execFileSync('journalctl',['--user','-u','plasma-kwin_wayland','--since','-1 minute','--no-pager','-o','cat'],{encoding:'utf8',maxBuffer:8*1024*1024});
            const line=log.split('\n').find(l=>l.includes(marker+' {'));
            if(line)return JSON.parse(line.slice(line.indexOf(marker)+marker.length+1));
        }
        throw Error('Compositor restore did not return');
    }finally {
        if(loaded)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',marker);
        fs.unlinkSync(file);fs.rmdirSync(dir);
    }
}
function compare(before,after) {
    const errors=[];
    for(const old of before.windows) {
        const now=after.windows.find(w=>w.id===old.id);
        if(!now){errors.push('A preexisting window disappeared');continue;}
        if(now.minimized!==old.minimized||now.hidden!==old.hidden)errors.push('Window visibility changed');
        if(old.fullScreen!==undefined&&now.fullScreen!==old.fullScreen)errors.push('Window fullscreen state changed');
        if(old.maximizeMode!==undefined&&now.maximizeMode!==old.maximizeMode)errors.push('Window maximize state changed');
        if(!old.minimized&&!old.hidden&&['x','y','width','height'].some(k=>Math.abs(now.rect[k]-old.rect[k])>1))errors.push('Visible window geometry changed');
    }
    if(JSON.stringify(before.screens)!==JSON.stringify(after.screens))errors.push('Output work areas changed');
    if(errors.length)throw Error(errors.join('; '));
    return before.windows.length;
}
module.exports={capture,compare,restore};
if(require.main===module)(async()=>{
    const [mode,file,permission]=process.argv.slice(2);
    if(!['--capture','--compare','--restore'].includes(mode)||!file)
        throw Error('Use --capture PRIVATE_FILE, --compare PRIVATE_FILE, or --restore PRIVATE_FILE --allow-window-moves');
    if(mode==='--restore') {
        if(permission!=='--allow-window-moves')throw Error('Restore requires --allow-window-moves');
        const result=await restore(JSON.parse(fs.readFileSync(file,'utf8')));
        console.log('Restored',result.restored,'windows; missing',result.missing);
        return;
    }
    const current=await capture();
    if(mode==='--capture'){fs.writeFileSync(file,JSON.stringify(current,null,2),{mode:0o600,flag:'wx'});console.log('Captured',current.windows.length,'windows locally');}
    else console.log('PASS preserved',compare(JSON.parse(fs.readFileSync(file,'utf8')),current),'windows and output work areas');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
