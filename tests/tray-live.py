"""Opt-in real Plasma tray actions; never closes application windows."""
import argparse
import json
import pathlib
import time
import dbus

parser=argparse.ArgumentParser()
parser.add_argument('--allow-settings-changes',action='store_true')
args=parser.parse_args()
if not args.allow_settings_changes: parser.error('Explicit desktop/settings authorization required')
bus=dbus.SessionBus()
watcher=dbus.Interface(bus.get_object('org.kde.StatusNotifierWatcher','/StatusNotifierWatcher'),'org.freedesktop.DBus.Properties')
service=None
for registered in watcher.Get('org.kde.StatusNotifierWatcher','RegisteredStatusNotifierItems'):
    name=str(registered).split('/')[0]
    props=dbus.Interface(bus.get_object(name,'/StatusNotifierItem'),'org.freedesktop.DBus.Properties')
    if str(props.Get('org.kde.StatusNotifierItem','Id'))=='com.userfirst.tilekeep': service=name;break
if not service: raise RuntimeError('Tilekeep tray not registered')
menu=dbus.Interface(bus.get_object(service,'/MenuBar'),'com.canonical.dbusmenu')
props=dbus.Interface(bus.get_object(service,'/StatusNotifierItem'),'org.freedesktop.DBus.Properties')

def entries():
    _,root=menu.GetLayout(0,-1,dbus.Array([],signature='s'))
    def walk(item,parent=''):
        ident,properties,children=item
        label=str(properties.get('label',''))
        yield int(ident),label,parent,properties
        for child in children: yield from walk(child,label)
    return list(walk(root))

def click(label,parent=None):
    found=[e for e in entries() if e[1]==label and (parent is None or e[2]==parent)]
    if len(found)!=1: raise RuntimeError(f'Ambiguous/missing menu entry: {label}: {found}')
    menu.Event(found[0][0],'clicked',dbus.Int32(0),dbus.UInt32(0))
    time.sleep(1.2)

def check(value,message):
    if not value: raise AssertionError(message)
    print('PASS',message,flush=True)

settings_dir=pathlib.Path.home()/'.config/tilekeep'
def settings(): return json.loads((settings_dir/'settings.json').read_text())
original_autostart=next(e[3].get('toggle-state',0) for e in entries() if e[1]=='Start with Linux')
try:
    for _ in range(30):
        if 'running' in str(props.Get('org.kde.StatusNotifierItem','Title')): break
        time.sleep(.2)
    check('running' in str(props.Get('org.kde.StatusNotifierItem','Title')),'tray reports running')
    click('Pause tiling');check('paused' in str(props.Get('org.kde.StatusNotifierItem','Title')),'pause changes real tray status')
    click('Pause tiling');check('running' in str(props.Get('org.kde.StatusNotifierItem','Title')),'resume changes real tray status')
    click('4 px');check(settings()['gap']==4,'gap menu persists 4 pixels')
    click('1 px');check(settings()['gap']==1,'gap menu returns to 1 pixel')
    click('Start with Linux')
    startup=pathlib.Path.home()/'.config/autostart/com.userfirst.tilekeep.desktop'
    check(startup.exists() != bool(original_autostart),'startup toggle changes the real login entry')
    click('Start with Linux');check(startup.exists()==bool(original_autostart),'startup entry restored')
    before=set((settings_dir/'snapshots').glob('*.json')) if (settings_dir/'snapshots').exists() else set()
    click('Save snapshot now')
    after=set((settings_dir/'snapshots').glob('*.json'))
    saved=after-before;check(len(saved)==1,'tray saves a real snapshot')
    snapshot=saved.pop();data=json.loads(snapshot.read_text())
    check(bool(data['windows']) and bool(data['monitors']),'snapshot contains applications and layout')
    click('4 px');click('Snapshot '+snapshot.stem,'Load snapshot')
    check(settings()['gap']==1,'loading snapshot restores its saved gap')
    check('running' in str(props.Get('org.kde.StatusNotifierItem','Title')),'snapshot restoration completed')
    click('Snapshot '+snapshot.stem,'Snapshot at startup')
    check(settings()['startup_snapshot']==snapshot.stem,'startup snapshot selection persists')
    click('None','Snapshot at startup');check(settings()['startup_snapshot'] is None,'startup snapshot can be disabled')
    click('Check for updates')
    check(any('signing key not configured' in e[1] for e in entries()),'unconfigured updater reports its real status')
    print('Saved test snapshot:',snapshot,flush=True)
finally:
    state=entries()
    if next(e[3].get('toggle-state',0) for e in state if e[1]=='Pause tiling'): click('Pause tiling')
    if next(e[3].get('toggle-state',0) for e in entries() if e[1]=='Start with Linux')!=original_autostart: click('Start with Linux')
