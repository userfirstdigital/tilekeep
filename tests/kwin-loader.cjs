// KWin assigns IDs by script count, which can collide after another script is
// unloaded. Reserve IDs without running anything until the next path is free.
function loadScript(dbus,file,plugin) {
    const paths=new Set(dbus().split('\n').map(s=>s.trim()));
    const reservations=[];
    try {
        for(let i=0;i<64;i++) {
            const name=`${plugin}-id-reservation-${process.pid}-${Date.now()}-${i}`;
            const id=Number(dbus('/Scripting','org.kde.kwin.Scripting.loadDeclarativeScript',file,name));
            if(id<0)throw Error('KWin refused ID reservation');
            reservations.push(name);
            if(!paths.has(`/Scripting/Script${id+1}`))break;
            if(i===63)throw Error('No free script ID');
        }
        const id=dbus('/Scripting','org.kde.kwin.Scripting.loadDeclarativeScript',file,plugin);
        if(Number(id)<0)throw Error('KWin refused script');
        dbus(`/Scripting/Script${id}`,'org.kde.kwin.Script.run');
        return id;
    } finally {
        for(const name of reservations)dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',name);
    }
}
module.exports={loadScript};
