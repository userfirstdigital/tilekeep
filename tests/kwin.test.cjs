// Run with: node --test tests/kwin.test.cjs
// Execute the actual compositor-side functions without touching the desktop.
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const test = require('node:test');
const assert = require('node:assert/strict');
const {loadScript}=require('./kwin-loader.cjs');

function backend() {
    const source = readFileSync(new URL('../src/linux/kwin.qml', `file://${__filename}`), 'utf8');
    const functions = source.slice(source.indexOf('    function rect('), source.indexOf('    Component.onCompleted:'));
    const timer = () => ({running:false, restart(){this.running=true;}, stop(){this.running=false;}});
    const context = vm.createContext({
        console:{log(){}}, gap:10, dryRun:false, enabled:true, paused:false, minRatio:.05, maxRatio:.95,
        monitors:[], focused:null, floating:new Set(), identities:new Map(), expectedGeometry:new Map(),
        deferredPlacements:new Map(), interactiveWindows:new Set(), placementQueue:[], currentPlacement:null,
        placementAttempt:0, windowConnections:new Map(), sequence:1,pendingSnapshot:null,
        previewOwner:null,previewGeometry:null,dragPreview:{visible:false},resizeGuide:{visible:false},resizeGuides:[],resizePreviewWindows:new Map(),pendingResizeEnds:new Map(),
        displayTransition:false,displayEpoch:0,displayFingerprint:'',displayStableTicks:0,displaySamples:[],pendingWindows:new Set(),deferredSnapshot:null,saveAfterDisplay:false,
        placementDeadline:timer(), placementSpacing:timer(), recoveryTimer:timer(),workAreaTimer:timer(),resizeFinishTimer:timer(),
        KWin:{MaximizeArea:0},
        Workspace:{currentDesktop:1,currentActivity:'test',raiseWindow(){},hideOutline(){}},
    });
    context.root=context;
    context.windowObserver={createObject(_parent,{target,handlers}){
        const links=[['frameGeometryChanged','geometry'],['interactiveMoveResizeStarted','started'],['interactiveMoveResizeStepped','stepped'],['interactiveMoveResizeFinished','finished'],['minimizedChanged','minimized'],['maximizedChanged','minimized'],['fullScreenChanged','minimized'],['desktopsChanged','minimized'],['activitiesChanged','minimized']];
        for(const [s,h] of links)target[s].connect(handlers[h]);
        const disconnect=()=>{for(const [s,h] of links)target[s].disconnect(handlers[h]);};
        return {set target(v){if(v===null)disconnect();},destroy:disconnect};
    }};
    context.Workspace.clientArea=(_option,output)=>context.monitors.find(m=>m.output===output).area;
    vm.runInContext(functions,context);
    return context;
}
function signal() {
    const handlers=new Set();
    return {connect:f=>handlers.add(f),disconnect:f=>handlers.delete(f),emit(...args){for(const f of handlers)f(...args);},handlers};
}
function window(c, {async=false}={}) {
    const w={managed:true, normalWindow:true, moveable:true, resizeable:true, caption:'test',
        desktopFileName:'test', output:1, desktops:[1], activities:[], setMaximize(){},
        frameGeometryChanged:signal(),interactiveMoveResizeStarted:signal(),interactiveMoveResizeStepped:signal(),
        interactiveMoveResizeFinished:signal(),minimizedChanged:signal(),maximizedChanged:signal(),
        fullScreenChanged:signal(),desktopsChanged:signal(),activitiesChanged:signal()};
    let geometry={x:0,y:0,width:100,height:100};
    Object.defineProperty(w,'frameGeometry',{get:()=>geometry,set:g=>{w.requested=g;if(!async){geometry=g;w.frameGeometryChanged.emit();}}});
    w.commit=()=>{geometry=w.requested;w.frameGeometryChanged.emit();};
    const m={output:1,area:{x:0,y:0,width:1000,height:800},root:c.leaf()};
    c.monitors.push(m);c.assign(m.root,w);c.connectWindow(w);
    return w;
}
function displayFixture(options={}) {
    const c=backend(),w=window(c,options),m=c.monitors[0];
    const output={name:'DP-3',enabled:true,area:{x:0,y:0,width:1000,height:800}};
    m.name=output.name;m.output=output;m.online=true;w.output=output;
    c.Workspace.screens=[output];c.Workspace.stackingOrder=[w];
    c.Workspace.clientArea=(_kind,o)=>o.area;
    c.Workspace.sendClientToScreen=(w,o)=>{w.output=o;};
    return {c,w,m,output};
}
function settleDisplays(c) {for(let i=0;i<5;i++)c.workAreasChanged();}
function arrange(c,m,pairs) {
    const items=pairs.map(([w,rect])=>{const slot=c.leaf();c.assign(slot,w);return {slot,rect};});
    m.root=c.layoutAround(items,c.inset(m.area),0);assert.ok(m.root,'fixture is a valid layout');
    for(const [w,r] of pairs)assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},r);
}
test('the observed DP-3 -> Placeholder-1 -> DP-3 wake sequence keeps the original tree and geometry',()=>{
    const {c,w,m,output}=displayFixture();c.apply();
    const tree=m.root,before={...w.frameGeometry};
    const placeholder={name:'Placeholder-1',enabled:true,area:{x:0,y:0,width:1920,height:1080}};
    c.Workspace.screens=[placeholder];w.output=placeholder;
    c.beginDisplayTransition();settleDisplays(c);
    assert.equal(c.displayTransition,true);assert.equal(m.root,tree);assert.equal(m.output,null);
    assert.equal(c.monitors.length,1);assert.equal(c.placements().length,0);
    // Simulate KWin itself relocating/shrinking the native client while disconnected.
    w.frameGeometry={x:50,y:50,width:300,height:250};
    const replacement={...output};c.Workspace.screens=[replacement];
    settleDisplays(c);
    assert.equal(c.displayTransition,false);assert.equal(m.root,tree);assert.equal(m.output,replacement);
    assert.deepEqual({...w.frameGeometry},before);assert.equal(w.output,replacement);
});
test('pending, retry and direct geometry writes are blocked before screensChanged arrives',()=>{
    const {c,w,m}=displayFixture({async:true});c.apply();assert.equal(c.currentPlacement.window,w);
    const tree=m.root,request=w.requested;
    c.Workspace.screens=[];
    c.placementTimedOut();c.retryDeferredPlacements();c.placeNextWindow();c.placeWindow(w,{x:1,y:1,width:1,height:1});c.apply();
    assert.equal(c.displayTransition,true);assert.equal(c.currentPlacement,null);assert.equal(c.placementQueue.length,0);
    assert.equal(c.expectedGeometry.size,0);assert.equal(c.deferredPlacements.size,0);
    assert.equal(w.requested,request);assert.equal(m.root,tree);
});
test('late panel work areas reset the wake debounce without scaling the retained tree early',()=>{
    const {c,w,m,output}=displayFixture();c.apply();const oldArea={...m.area},tree=m.root;
    c.beginDisplayTransition();c.Workspace.screens=[];settleDisplays(c);
    const returning={...output,area:{...output.area,height:830}};c.Workspace.screens=[returning];
    for(let i=0;i<4;i++)c.workAreasChanged();
    assert.equal(c.displayTransition,true);assert.deepEqual({...m.area},oldArea);
    returning.area={...output.area};
    for(let i=0;i<4;i++)c.workAreasChanged();
    assert.equal(c.displayTransition,true);assert.equal(m.root,tree);
    c.workAreasChanged();assert.equal(c.displayTransition,false);
    assert.deepEqual({...m.area},oldArea);assert.equal(m.root,tree);
});
test('clients opened and closed with no real monitor are reconciled once without stale references',()=>{
    const {c,w,m,output}=displayFixture();c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
    const newcomer={managed:true,normalWindow:true,moveable:true,resizeable:true,caption:'new',output,frameGeometry:{x:1,y:1,width:100,height:100},setMaximize(){}};
    c.appeared(newcomer);c.appeared(newcomer);
    assert.equal(c.pendingWindows.size,1);assert.equal(c.slotOf(newcomer),null);
    c.vanished(w);w.deleted=true;c.Workspace.stackingOrder=[newcomer];
    c.Workspace.screens=[{...output}];settleDisplays(c);
    assert.equal(c.pendingWindows.size,0);assert.equal(c.slotOf(w),null);
    assert.equal(c.allWindows(m.root).filter(x=>x===newcomer).length,1);
});
test('a runtime starting on a placeholder waits for a real monitor before enrolling windows',()=>{
    const {c,w,output}=displayFixture();c.monitors=[];
    c.Workspace.screens=[{name:'Placeholder-1',area:{x:0,y:0,width:1920,height:1080}}];
    c.syncMonitors();c.appeared(w);assert.equal(c.displayTransition,true);assert.equal(c.monitors.length,0);
    c.Workspace.screens=[output];settleDisplays(c);
    assert.equal(c.monitors.length,1);assert.ok(c.slotOf(w));assert.equal(c.monitors[0].name,'DP-3');
});
test('removing one of two monitors keeps both roots and restores the same membership on reconnect',()=>{
    const {c,w,m,output}=displayFixture();
    const secondOutput={name:'HDMI-A-1',enabled:true,area:{x:1000,y:0,width:1200,height:900}};
    const second={name:secondOutput.name,output:secondOutput,area:secondOutput.area,online:true,root:c.leaf()};
    c.monitors.push(second);c.Workspace.screens=[output,secondOutput];const tree=m.root,otherTree=second.root;
    c.Workspace.screens=[secondOutput];c.beginDisplayTransition();settleDisplays(c);
    assert.equal(m.online,false);assert.equal(m.root,tree);assert.equal(second.root,otherTree);assert.equal(c.slotOf(w)[0],m);
    assert.equal(c.monitorForOutput(output),second);assert.equal(c.placements().length,0);
    c.Workspace.screens=[secondOutput,{...output}];c.beginDisplayTransition();settleDisplays(c);
    assert.equal(m.online,true);assert.equal(m.root,tree);assert.equal(second.root,otherTree);assert.equal(c.slotOf(w)[0],m);
});
test('repeated disconnects and replacement output objects do not accumulate monitors or duplicate windows',()=>{
    const {c,w,m,output}=displayFixture();const tree=m.root;
    for(let i=0;i<12;i++) {
        c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
        c.Workspace.screens=[{...output}];settleDisplays(c);
        assert.equal(c.monitors.length,1);assert.equal(m.root,tree);assert.equal(c.allWindows(tree).filter(x=>x===w).length,1);
    }
});
test('a drag interrupted by unplugging cannot overwrite the preserved layout when Finished arrives late',()=>{
    const {c,w,m,output}=displayFixture();c.apply();const tree=m.root,before={...w.frameGeometry};w.move=false;
    w.interactiveMoveResizeStarted.emit();
    c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
    w.frameGeometry={x:100,y:100,width:400,height:400};
    c.Workspace.screens=[{...output}];settleDisplays(c);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.equal(m.root,tree);assert.deepEqual({...w.frameGeometry},before);
});
test('paused tiling stays paused across reconnect and resumes with the original layout',()=>{
    const {c,w,m,output}=displayFixture();c.apply();const before={...w.frameGeometry},tree=m.root;c.setPaused(true);
    c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
    w.frameGeometry={x:50,y:50,width:200,height:200};c.Workspace.screens=[{...output}];settleDisplays(c);
    assert.equal(c.paused,true);assert.equal(w.frameGeometry.width,200);assert.equal(m.root,tree);
    c.setPaused(false);assert.deepEqual({...w.frameGeometry},before);
});
test('native resize steps arriving before screensChanged cannot write geometry or alter the tree',()=>{
    const {c,w,m}=displayFixture();c.apply();const tree=m.root,before={...w.frameGeometry};w.move=false;
    w.interactiveMoveResizeStarted.emit();c.Workspace.screens=[];
    w.interactiveMoveResizeStepped.emit({...before,width:before.width-100});
    assert.equal(c.displayTransition,true);assert.deepEqual({...w.frameGeometry},before);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();assert.equal(m.root,tree);
    assert.equal(c.slotAt({x:50,y:50}),null);assert.equal(c.interactiveWindows.size,0);
});
test('invalid and disabled outputs never replace a saved real-monitor area',()=>{
    const {c,m,output}=displayFixture();const before={...m.area};
    for(const bad of [{...output,enabled:false},{...output,area:{x:0,y:0,width:0,height:0}},{...output,area:{x:0,y:0,width:NaN,height:800}}]) {
        c.Workspace.screens=[bad];c.beginDisplayTransition();settleDisplays(c);
        assert.equal(c.displayTransition,true);assert.deepEqual({...m.area},before);
    }
});
test('snapshot save and load requests wait until monitor recovery is stable',()=>{
    const {c,output}=displayFixture();c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
    let saved=0,loaded=null;c.snapshotSave={call(){saved++;}};
    c.saveSnapshot();c.restoreSnapshot('pending snapshot');assert.equal(saved,0);assert.equal(c.deferredSnapshot,'pending snapshot');
    c.restoreSnapshot=s=>{loaded=s;};c.Workspace.screens=[{...output}];settleDisplays(c);
    assert.equal(loaded,'pending snapshot');assert.equal(saved,1);assert.equal(c.saveAfterDisplay,false);
});
test('snapshot matching never duplicates an offline monitor tree onto an unmatched online output',()=>{
    const {c,w,m,output}=displayFixture();w.internalId='a';
    const saved={gap:1,windows:[{token:'a',app:'test',title:'test',rect:w.frameGeometry}],monitors:[{name:m.name,root:c.snapshotTree(m.root)}]};
    const other={name:'HDMI-A-1',enabled:true,area:{x:0,y:0,width:1200,height:900}};
    c.Workspace.screens=[other];c.beginDisplayTransition();settleDisplays(c);
    c.snapshotRestored={call(){}};c.restoreSnapshot(JSON.stringify(saved));
    assert.equal(c.monitors.length,2);assert.equal(c.slotOf(w)[0],m);assert.equal(m.online,false);
    assert.equal(c.monitors.flatMap(m=>c.allWindows(m.root)).filter(x=>x===w).length,1);
    assert.equal(c.placements().length,0);
    c.Workspace.screens=[other,{...output}];c.beginDisplayTransition();settleDisplays(c);
    assert.equal(c.slotOf(w)[0],m);assert.equal(c.placements().length,1);
});
test('a pending client closed while offline is not retained, and minimized and floating windows remain untouched',()=>{
    const {c,w,m,output}=displayFixture();w.minimized=true;const before={...w.frameGeometry};
    const extra={...w,minimized:false,internalId:'floating',frameGeometry:{x:25,y:30,width:200,height:200}};
    c.floating.add(extra);c.Workspace.stackingOrder.push(extra);
    c.Workspace.screens=[];c.beginDisplayTransition();settleDisplays(c);
    const pending={...extra,internalId:'pending'};c.appeared(pending);assert.equal(c.pendingWindows.size,1);
    c.vanished(pending);pending.deleted=true;assert.equal(c.pendingWindows.size,0);
    c.Workspace.screens=[{...output}];settleDisplays(c);
    assert.equal(w.minimized,true);assert.deepEqual({...w.frameGeometry},before);
    assert.equal(c.floating.has(extra),true);assert.equal(c.slotOf(extra),null);
    assert.deepEqual({...extra.frameGeometry},{x:25,y:30,width:200,height:200});
    assert.equal(c.allWindows(m.root).length,1);
});
test('synchronous geometry completion disarms its deadline',()=>{
    const c=backend();const w=window(c);c.apply();
    assert.equal(c.currentPlacement,null);assert.equal(c.placementDeadline.running,false);
    assert.equal(w.frameGeometry.width,980);
});
test('sleeping clients are reconciled after wake without another retile command',()=>{
    const c=backend();const w=window(c,{async:true});c.apply();
    for(let i=0;i<3;i++)c.placementTimedOut();
    assert.equal(c.deferredPlacements.size,1);assert.equal(c.currentPlacement,null);
    c.retryDeferredPlacements();w.commit();
    assert.equal(c.currentPlacement,null);assert.equal(c.deferredPlacements.size,0);
    assert.equal(w.frameGeometry.width,980);
});
test('native KWin tile associations are released even when geometry already matches',()=>{
    const c=backend(),w=window(c);c.apply();let released=0;
    w.tile={unmanage(client){assert.equal(client,w);released++;w.tile=null;}};
    c.apply();assert.equal(released,1);assert.equal(w.tile,null);
});
test('interactive moves cancel pending requests and never resize with undefined geometry',()=>{
    const c=backend();const w=window(c,{async:true});c.apply();
    w.interactiveMoveResizeStarted.emit();c.placementTimedOut();c.retryDeferredPlacements();c.apply();
    assert.equal(c.currentPlacement,null);assert.equal(c.expectedGeometry.size,0);
    assert.equal(c.interactiveWindows.size,1);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();assert.equal(c.interactiveWindows.size,0);
});
test('removal of an in-flight window clears pending and signal references',()=>{
    const c=backend();const w=window(c,{async:true});c.apply();
    c.disconnectWindow(w);c.vanished(w);c.placementTimedOut();
    assert.equal(c.currentPlacement,null);assert.equal(c.expectedGeometry.size,0);
    assert.equal(w.frameGeometryChanged.handlers.size,0);assert.equal(c.windowConnections.size,0);
});
test('nested split resize has internal bounds and preserves valid ratios',()=>{
    const c=backend();const w=window(c);const m=c.monitors[0];
    c.splitSlot(m,m.root,'x',false,{caption:'second'});
    c.splitSlot(m,m.root.first,'y',false,{caption:'third'});
    const before=c.rects(m).get(c.slotOf(w)[1]);
    c.adjustRatio(w,before,{...before,width:before.width+100});
    assert.ok(m.root.ratio>.5&&m.root.ratio<1);
});
test('drop corner uses the dominant axis, not unconditional left/right priority',()=>{
    const c=backend();const r={x:0,y:0,width:100,height:100};
    assert.equal(c.zone(r,{x:20,y:1}),'top');assert.equal(c.zone(r,{x:1,y:20}),'left');
    assert.equal(c.zone(r,{x:50,y:50}),'center');
});
test('minimized or floating deferred windows are not moved on recovery',()=>{
    for(const mode of ['minimized','floating']) {
        const c=backend();const w=window(c,{async:true});c.apply();
        for(let i=0;i<3;i++)c.placementTimedOut();
        if(mode==='minimized')w.minimized=true;else c.floating.add(w);
        c.retryDeferredPlacements();assert.equal(c.currentPlacement,null);
    }
});
test('maximized, fullscreen and security-prompt windows are left alone',()=>{
    const c=backend();const w=window(c);
    w.maximizeMode=3;assert.equal(c.visibleHere(w),false);
    w.maximizeMode=0;w.fullScreen=true;assert.equal(c.visibleHere(w),false);
    w.fullScreen=false;w.desktopFileName='org.kde.kwin.eisprompter';assert.equal(c.tileable(w),false);
});
test('Escape-cancelled moves do not drop onto the slot still under the cursor',()=>{
    const c=backend();const w=window(c);c.apply();
    let dropped=false;c.drop=()=>{dropped=true;};w.move=true;
    w.interactiveMoveResizeStarted.emit();
    // KWin has restored the original rectangle when Finished is emitted.
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.equal(dropped,false);assert.equal(c.interactiveWindows.size,0);
});
test('dragging an excluded prompt never enrolls it in the tiling tree',()=>{
    const c=backend();const w=window(c);c.detach(w);w.desktopFileName='org.kde.kwin.eisprompter';w.move=true;
    let dropped=false;c.drop=()=>{dropped=true;};
    w.interactiveMoveResizeStarted.emit();w.frameGeometry={x:200,y:200,width:300,height:200};
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.equal(dropped,false);assert.equal(c.slotOf(w),null);
});
test('split allocation honors client minimum sizes including decorations',()=>{
    const c=backend();const w=window(c);w.minSize={width:200,height:120};
    w.clientGeometry={x:0,y:20,width:100,height:80};
    const other={minSize:{width:200,height:620}};const m=c.monitors[0];
    c.splitSlot(m,m.root,'y',false,other);
    const rects=c.rects(m), first=rects.get(c.slotOf(w)[1]),second=rects.get(c.slotOf(other)[1]);
    assert.ok(first.height>=140);assert.equal(second.height,620);
    assert.equal(first.y+first.height+c.gap,second.y);
    assert.ok(second.y+second.height<=m.area.height-c.gap);
});
test('an impossible split stacks in the target rather than overlapping its neighbor',()=>{
    const c=backend();const w=window(c);w.minSize={width:800,height:600};
    const m=c.monitors[0],other={minSize:{width:800,height:600}};
    c.splitSlot(m,m.root,'y',false,other);
    assert.equal(m.root.kind,'leaf');assert.equal(m.root.windows.length,2);
    assert.equal(c.slotOf(w)[1],c.slotOf(other)[1]);
});
test('drop outline predicts minimum-constrained placement and the stack fallback',()=>{
    const c=backend();const target=window(c);target.minSize={width:200,height:140};
    const m=c.monitors[0],r=c.rects(m).get(m.root),w={minSize:{width:200,height:620}};
    const preview=c.dropPreview(w,[m,m.root,r],'bottom');
    c.splitSlot(m,m.root,'y',false,w);
    assert.deepEqual({...preview},{...c.rects(m).get(c.slotOf(w)[1])});
    const slot=c.slotOf(w)[1],slotRect=c.rects(m).get(slot);
    const stacked=c.dropPreview({minSize:{width:200,height:620}},[m,slot,slotRect],'top');
    assert.deepEqual({...stacked},{...slotRect});
});
test('stack shortcut does not collide with Spectacle or reuse the old saved binding',()=>{
    const source=readFileSync(new URL('../src/linux/kwin.qml', `file://${__filename}`),'utf8');
    assert.match(source,/name: "TilekeepStackAtCursor";[^\n]*sequence: "Meta\+Shift\+G"/);
    assert.doesNotMatch(source,/name: "TilekeepStack";/);
});
test('drag preview stays visible when KWin clears its shared outline on every step',()=>{
    const c=backend();const w=window(c);c.apply();w.move=true;
    c.Workspace.cursorPos={x:500,y:400};
    let shows=0;let visible=false;
    Object.defineProperty(c.dragPreview,'visible',{get:()=>visible,set:v=>{if(v)shows++;visible=v;}});
    w.interactiveMoveResizeStarted.emit();
    for(let i=0;i<100;i++) {
        w.interactiveMoveResizeStepped.emit();
        c.Workspace.hideOutline(); // KWin does this after the script's callback.
        assert.equal(c.dragPreview.visible,true);
    }
    assert.equal(shows,1);
    const first=c.previewGeometry;
    w.interactiveMoveResizeStepped.emit();assert.equal(c.previewGeometry,first);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();assert.equal(c.dragPreview.visible,false);
});
test('preview target changes move the existing surface without hiding it',()=>{
    const c=backend();const w=window(c);
    c.showPreview(w,{x:10,y:10,width:400,height:300});
    let hides=0;
    Object.defineProperty(c.dragPreview,'visible',{get:()=>true,set:v=>{if(!v)hides++;}});
    c.showPreview(w,{x:420,y:10,width:570,height:780});
    assert.equal(hides,0);assert.equal(c.dragPreview.x,420);assert.equal(c.dragPreview.width,570);
});
test('preview is dismissed on leaving outputs or removing its owner, not by another window',()=>{
    const c=backend();const w=window(c);w.move=true;
    c.Workspace.cursorPos={x:500,y:400};
    w.interactiveMoveResizeStarted.emit();w.interactiveMoveResizeStepped.emit();
    c.hidePreview({});assert.equal(c.dragPreview.visible,true);
    c.Workspace.cursorPos={x:2000,y:400};w.interactiveMoveResizeStepped.emit();
    assert.equal(c.dragPreview.visible,false);
    c.showPreview(w,{x:10,y:10,width:400,height:300});c.vanished(w);
    assert.equal(c.dragPreview.visible,false);assert.equal(c.previewOwner,null);
});
test('preview is an independent non-focusable surface, never the shared KWin outline',()=>{
    const source=readFileSync(new URL('../src/linux/kwin.qml', `file://${__filename}`),'utf8');
    assert.doesNotMatch(source,/Workspace\.(showOutline|hideOutline)\(/);
    assert.match(source,/transientParent: null/);
    assert.match(source,/Qt\.WindowTransparentForInput/);
    assert.match(source,/Qt\.WindowDoesNotAcceptFocus/);
});
test('invisible windows do not constrain resizing into apparently empty space',()=>{
    for(const state of [{minimized:true},{hidden:true},{desktops:[2]},{activities:['elsewhere']}]) {
        const c=backend(),w=window(c),m=c.monitors[0];
        const hidden={minSize:{width:450,height:600},...state};
        c.splitSlot(m,m.root,'x',false,hidden);
        const before=c.rects(m).get(c.slotOf(w)[1]);
        c.adjustRatio(w,before,{...before,width:850});
        assert.equal(c.rects(m).get(c.slotOf(w)[1]).width,850);
        assert.equal(c.minimumSize(c.slotOf(hidden)[1]).width,0);
    }
});
test('edge drops into minimized slots select halves and preserve hidden occupants',()=>{
    for(const z of ['left','right','top','bottom']) {
        const c=backend(),w=window(c),m=c.monitors[0];
        const hidden={minimized:true,minSize:{width:450,height:600}};
        c.splitSlot(m,m.root,'x',false,hidden);
        const source=c.slotOf(w)[1],target=c.slotOf(hidden)[1],r=c.rects(m).get(target);
        const p={x:r.x+r.width/2,y:r.y+r.height/2};
        if(z==='left')p.x=r.x+1;if(z==='right')p.x=r.x+r.width-1;
        if(z==='top')p.y=r.y+1;if(z==='bottom')p.y=r.y+r.height-1;
        const preview=c.dropPreview(w,[m,target,r],z);
        assert.ok(c.area(preview)<c.area(r));
        assert.equal(c.drop(w,p,false),true);
        assert.equal(c.slotOf(w)[1],target);assert.equal(c.slotOf(hidden)[1],source);
        assert.equal(hidden.minimized,true);assert.equal(c.leaves(m.root).length,3);
        assert.deepEqual({...c.rects(m).get(target)},{...preview});
        hidden.minimized=false;
        assert.ok(c.rects(m).get(source).width>=450);
    }
});
test('a center drop fills a genuinely empty slot without splitting it',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={};
    c.splitSlot(m,m.root,'x',false,other);
    const target=c.slotOf(other)[1],r=c.rects(m).get(target);c.detach(other);
    assert.equal(c.drop(w,{x:r.x+r.width/2,y:r.y+r.height/2},false),true);
    assert.equal(c.slotOf(w)[1],target);assert.equal(c.leaves(m.root).length,2);
});
test('all nine empty-space drop zones match their final placement',()=>{
    for(const x of [.1,.5,.9])for(const y of [.1,.5,.9]) {
        const c=backend(),w=window(c),m=c.monitors[0],other={};
        c.splitSlot(m,m.root,'x',false,other);const target=c.slotOf(other)[1];c.detach(other);
        const r=c.rects(m).get(target),p={x:r.x+r.width*x,y:r.y+r.height*y};
        const z=c.emptyZone(r,p),preview=c.dropPreview(w,[m,target,r],z);
        assert.equal(c.drop(w,p,false),true);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...preview},z);
        assert.equal(c.leaves(m.root).length,2+Number(x!==.5)+Number(y!==.5));
    }
});
test('a dragged window can take a quarter or half of its own vacated slot',()=>{
    for(const x of [.1,.5,.9])for(const y of [.1,.5,.9]) {
        const c=backend(),w=window(c),m=c.monitors[0],target=c.slotOf(w)[1];
        const r=c.rects(m).get(target),p={x:r.x+r.width*x,y:r.y+r.height*y};
        const z=c.emptyZone(r,p),wanted=c.emptyPart(r,z),tree=m.root;
        const preview=c.dropPreview(w,[m,target,r],z);
        assert.equal(m.root,tree,'hover must not edit the saved layout');
        assert.deepEqual({...preview},{...wanted},z);
        assert.equal(c.drop(w,p,false),z!=='center');
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...wanted},z);
    }
});
test('native move callbacks keep full guides and change fractions inside the source slot without flicker',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.apply();w.move=true;
    const before={...w.frameGeometry};c.Workspace.cursorPos={x:200,y:20};
    w.interactiveMoveResizeStarted.emit();
    for(const [x,y,z] of [[.1,.1,'top-left'],[.5,.1,'top'],[.9,.9,'bottom-right'],[.5,.5,'center']]) {
        c.Workspace.cursorPos={x:before.x+before.width*x,y:before.y+before.height*y};
        w.frameGeometry={...before,x:before.x+30,y:before.y+40};
        w.interactiveMoveResizeStepped.emit(w.frameGeometry);
        assert.equal(c.previewZone,z);assert.equal(c.dragPreview.visible,true);
        assert.deepEqual({...c.previewArea},before);
        assert.deepEqual({...c.previewGeometry},{...c.emptyPart(before,z)});
    }
    // Escape: no fractional split is committed by merely hovering over it.
    const tree=m.root;w.frameGeometry=before;w.interactiveMoveResizeFinished.emit();
    assert.equal(m.root,tree);assert.equal(c.dragPreview.visible,false);assert.equal(c.interactiveWindows.size,0);
});
test('source-slot fractions preserve hidden occupants and never reinterpret a visible stack as empty',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};
    c.assign(m.root,hidden);const source=m.root,r=c.rects(m).get(source);
    const p={x:r.x+r.width*.1,y:r.y+r.height*.1};
    assert.equal(c.vacantForDrop(source,w),true);
    assert.equal(c.drop(w,p,false),true);
    assert.equal(c.allWindows(m.root).filter(other=>other===w).length,1);
    assert.equal(c.allWindows(m.root).filter(other=>other===hidden).length,1);
    assert.equal(hidden.minimized,true);
    const visible={...w};c.assign(c.slotOf(w)[1],visible);
    assert.equal(c.vacantForDrop(c.slotOf(w)[1],w),false);
});
test('undersized quarters fall back to a fitting half or full area',()=>{
    const c=backend(),w=window(c);w.minSize={width:400,height:300};
    const r={x:0,y:0,width:600,height:800};
    assert.equal(c.fittingEmptyZone(w,r,'top-left'),'top');
    w.minSize={width:400,height:600};assert.equal(c.fittingEmptyZone(w,r,'top-left'),'center');
    w.minSize={width:700,height:600};assert.equal(c.fittingEmptyZone(w,r,'top-left'),'unavailable');
});
test('full-space hover occupies the central 70 percent, with narrow edge and corner targets',()=>{
    const c=backend(),r={x:-500,y:30,width:1000,height:800};
    for(const x of [.15,.2,.5,.8,.85])for(const y of [.15,.2,.5,.8,.85])
        assert.equal(c.emptyZone(r,{x:r.x+x*r.width,y:r.y+y*r.height}),'center',`${x},${y}`);
    for(const [x,y,z] of [[.149,.5,'left'],[.851,.5,'right'],[.5,.149,'top'],[.5,.851,'bottom'],[.149,.149,'top-left'],[.851,.851,'bottom-right']])
        assert.equal(c.emptyZone(r,{x:r.x+x*r.width,y:r.y+y*r.height}),z);
});
test('minimum fit uses application hints plus decorations, not its current large size',()=>{
    const c=backend(),w=window(c);w.minSize={width:200,height:100};
    w.frameGeometry={x:0,y:0,width:1000,height:800};w.clientGeometry={x:4,y:30,width:992,height:766};
    const r={x:10,y:10,width:430,height:280}; // quarter = 210x135, minimum frame = 208x134
    assert.equal(c.fittingEmptyZone(w,r,'top-left'),'top-left');
    w.minSize.width=203; // minimum frame width 211 no longer fits the quarter
    assert.equal(c.fittingEmptyZone(w,r,'top-left'),'top');
    w.minSize.height=250; // even the full frame is now too short
    assert.equal(c.fittingEmptyZone(w,r,'top-left'),'unavailable');
});
test('an undersized vacancy rejects the drop without rearranging neighbors',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={};
    w.minSize={width:600,height:500};c.splitSlot(m,m.root,'x',false,other);
    const target=c.slotOf(other)[1];c.detach(other);const r=c.rects(m).get(target),root=m.root;
    assert.equal(c.drop(w,{x:r.x+r.width/2,y:r.y+r.height/2},false),false);
    assert.equal(m.root,root);assert.equal(target.windows.length,0);
});
test('randomized empty-space previews preserve trees and match committed geometry',()=>{
    let seed=41;const random=()=>((seed=Math.imul(seed,1664525)+1013904223>>>0)/4294967296);
    for(let i=0;i<180;i++) {
        const c=backend(),w=window(c),m=c.monitors[0],other={minimized:i%2===0};
        m.area={x:-1200,y:20,width:800+Math.floor(random()*3200),height:700+Math.floor(random()*1300)};c.gap=Math.floor(random()*17);
        c.splitSlot(m,m.root,'x',false,other);m.root.ratio=i%9===0?.05:.15+random()*.65;
        const target=c.slotOf(other)[1];if(i%2)c.detach(other);
        const r=c.rects(m).get(target),p={x:r.x+r.width*random(),y:r.y+r.height*random()};
        const z=c.emptyZone(r,p),tree=m.root,seq=c.sequence,preview=c.dropPreview(w,[m,target,r],z);
        assert.equal(m.root,tree);assert.equal(c.sequence,seq);
        assert.equal(c.drop(w,p,false),true);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...preview},'iteration '+i);
        const windows=c.allWindows(m.root);assert.equal(new Set(windows).size,windows.length);
    }
});
test('new windows use minimized vacancies without displacing visible neighbors',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};
    c.splitSlot(m,m.root,'x',false,hidden);const existing=c.slotOf(w)[1],vacancy=c.slotOf(hidden)[1];
    const before=c.rects(m).get(existing),newWindow={...w,desktopFileName:'new'};
    c.focused=w;assert.equal(c.appeared(newWindow),true);
    assert.equal(c.slotOf(newWindow)[1],vacancy);assert.equal(existing.windows.length,1);
    assert.deepEqual({...c.rects(m).get(existing)},{...before});assert.equal(hidden.minimized,true);
});
test('unstack separates overlapping windows into visible free space',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};
    c.splitSlot(m,m.root,'x',false,hidden);
    const second={...w,desktopFileName:'second'};c.assign(c.slotOf(w)[1],second);
    assert.equal(c.unstack(second),true);
    assert.notEqual(c.slotOf(w)[1],c.slotOf(second)[1]);
    assert.equal(c.slotOf(w)[1].windows.length,1);
});
test('a usable vacancy wins over a collapsed remembered slot',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],gone={};
    c.splitSlot(m,m.root,'x',false,gone);const remembered=c.slotOf(gone)[1];c.detach(gone);
    remembered.remembered='new';m.root.ratio=.95;
    const open=c.leaf(),old=m.root;m.root={kind:'split',axis:'y',ratio:.5,first:old,second:open,parent:null};old.parent=m.root;open.parent=m.root;
    const incoming={...w,desktopFileName:'new'};c.appeared(incoming);
    assert.equal(c.slotOf(incoming)[1],open);assert.equal(remembered.windows.length,0);
});
test('new windows split another fitting slot instead of stacking on a cramped focus',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={minSize:{width:50,height:50}};
    w.minSize={width:200,height:600};c.splitSlot(m,m.root,'x',false,other);m.root.ratio=.3;
    const focusSlot=c.slotOf(w)[1];c.focused=w;
    const incoming={...w,desktopFileName:'new',minSize:{width:200,height:500}};c.appeared(incoming);
    assert.equal(focusSlot.windows.length,1);assert.notEqual(c.slotOf(incoming)[1],c.slotOf(other)[1]);
});
test('empty-space guides keep full bounds while the selected quarter changes',()=>{
    const c=backend(),w=window(c),full={x:10,y:20,width:800,height:600};
    c.showPreview(w,c.emptyPart(full,'top-left'),full,'top-left');
    assert.equal(c.dragPreview.width,800);assert.equal(c.previewGeometry.width,395);
    c.showPreview(w,c.emptyPart(full,'bottom-right'),full,'bottom-right');
    assert.equal(c.dragPreview.visible,true);assert.equal(c.dragPreview.x,10);
    assert.equal(c.previewZone,'bottom-right');c.hidePreview();assert.equal(c.previewArea,null);
});
test('live loader bypasses reused KWin IDs without running or unloading other scripts',()=>{
    for(const ids of [[],[1],[0,2,3],[0,4,5]]) {
        const scripts=new Map(ids.map(id=>[`other-${id}`,id]));
        const exported=new Map(ids.map(id=>[id,`other-${id}`]));
        const ran=[];
        const dbus=(path,method,file,name)=>{
            if(!path)return [...exported.keys()].map(id=>`/Scripting/Script${id}`).join('\n');
            if(method.endsWith('.loadDeclarativeScript')) {
                const id=scripts.size;scripts.set(name,id);
                if(!exported.has(id))exported.set(id,name);
                return String(id);
            }
            if(method.endsWith('.unloadScript')) {
                assert.ok(!file.startsWith('other-'));
                const id=scripts.get(file);scripts.delete(file);
                if(exported.get(id)===file)exported.delete(id);
                return 'true';
            }
            if(method.endsWith('.run')) {ran.push(exported.get(Number(path.split('Script').pop())));return '';}
            throw Error('Unexpected call');
        };
        loadScript(dbus,'test.qml','tilekeep-test');
        assert.deepEqual(ran,['tilekeep-test']);assert.equal(scripts.size,ids.length+1);
        for(const id of ids)assert.equal(exported.get(id),`other-${id}`);
    }
});
test('old resize stops absorb a vacant strip and its gap on every side',()=>{
    for(const axis of ['x','y'])for(const first of [true,false])for(const minimized of [true,false]) {
        const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};
        c.splitSlot(m,m.root,axis,first,hidden);
        const vacancy=c.slotOf(hidden)[1];if(!minimized)c.detach(hidden);
        m.root.ratio=first?.05:.95;
        const map=c.rects(m),r=map.get(c.slotOf(w)[1]);
        assert.deepEqual({...r},{...c.inset(m.area)});
        assert.equal(c.area(map.get(vacancy)),0);
        assert.equal(c.slotAt({x:r.x,y:r.y})[1],c.slotOf(w)[1]);
        assert.equal(c.slotAt({x:r.x+r.width-1,y:r.y+r.height-1})[1],c.slotOf(w)[1]);
        if(minimized)assert.equal(c.slotOf(hidden)[1],vacancy);
    }
});
test('Chromium above a collapsed vacancy meets Dolphin with only the configured gap',()=>{
    const c=backend(),browser=window(c),m=c.monitors[0],dolphin={},hidden={minimized:true};
    m.area={x:1108,y:0,width:1947,height:1728};
    c.splitSlot(m,m.root,'y',false,dolphin);
    c.splitSlot(m,c.slotOf(browser)[1],'y',false,hidden);
    c.slotOf(browser)[1].parent.ratio=.95;
    const map=c.rects(m),a=map.get(c.slotOf(browser)[1]),b=map.get(c.slotOf(dolphin)[1]);
    assert.equal(a.height,849);assert.equal(b.y-c.rectBottom(a),10);
});
test('vacant-edge resize can reach the boundary and later reopen the space',()=>{
    for(const axis of ['x','y'])for(const first of [true,false]) {
        const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};
        c.splitSlot(m,m.root,axis,first,hidden);
        const before=c.rects(m).get(c.slotOf(w)[1]),full=c.inset(m.area);
        c.adjustRatio(w,before,full);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...full});
        assert.ok(c.slotOf(hidden),'hidden window remains tracked');
        c.adjustRatio(w,full,before);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...before});
    }
});
test('resizing into empty space does not move unrelated windows across an ancestor divider',()=>{
    for(const axis of ['x','y'])for(const first of [true,false]) {
        const c=backend(),w=window(c),m=c.monitors[0],unrelated={},empty={};
        c.gap=1;
        c.splitSlot(m,m.root,axis,first,empty);
        c.splitSlot(m,c.slotOf(w)[1],axis==='x'?'y':'x',false,unrelated);
        c.detach(empty);
        const original=c.rects(m),before=original.get(c.slotOf(w)[1]),fixed=original.get(c.slotOf(unrelated)[1]),after={...before};
        // A visible stretch of empty space breaks the otherwise aligned edge.
        if(axis==='x'){fixed.y+=60;fixed.height-=60;}else{fixed.x+=60;fixed.width-=60;}
        arrange(c,m,[[w,{...before}],[unrelated,{...fixed}]]);
        const edge=axis==='x'?(first?'left':'right'):(first?'top':'bottom');
        c.setEdge(after,edge,c.edgePosition(before,edge)+(first?-150:150));
        assert.equal(c.adjustRatio(w,before,after),true);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
        assert.deepEqual({...c.rects(m).get(c.slotOf(unrelated)[1])},{...fixed});
        const newcomer={};c.Workspace.activeWindow=w;
        c.appeared(newcomer);
        assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after,'new window cannot reclaim occupied resize area');
    }
});
test('connected windows below and across the shared edge follow in both directions, but separated aligned windows stay fixed',()=>{
    for(const delta of [-100,100]) {
        const c=backend(),w=window(c),m=c.monitors[0],below={},neighbor={},unrelated={};c.gap=1;
        const before={x:1,y:1,width:499,height:250},bottom={x:1,y:252,width:499,height:250};
        const across={x:501,y:1,width:498,height:501},fixed={x:1,y:600,width:499,height:199};
        arrange(c,m,[[w,before],[below,bottom],[neighbor,across],[unrelated,fixed]]);
        const after={...before,width:before.width+delta};assert.equal(c.adjustRatio(w,before,after),true);
        const next=c.rects(m);
        assert.deepEqual({...next.get(c.slotOf(unrelated)[1])},fixed);
        assert.deepEqual({...next.get(c.slotOf(w)[1])},after);
        assert.deepEqual({...next.get(c.slotOf(below)[1])},{...bottom,width:bottom.width+delta});
        assert.deepEqual({...next.get(c.slotOf(neighbor)[1])},{...across,x:across.x+delta,width:across.width-delta});
    }
});
test('shrinking a monitor-filling window creates reusable space without a parent split',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],before=c.rects(m).get(m.root);
    const after={...before,width:before.width-210};
    assert.equal(c.adjustRatio(w,before,after),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
    assert.ok(c.leaves(m.root).some(s=>!s.windows.length));
});
test('all four shared-edge directions pull and push a connected grid without depending on tree ancestry',()=>{
    for(const edge of ['left','right','top','bottom'])for(const delta of [-90,90]) {
        const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;
        const rs=[{x:1,y:1,width:499,height:399},{x:501,y:1,width:498,height:399},{x:1,y:401,width:499,height:398},{x:501,y:401,width:498,height:398}];
        const index=edge==='left'?1:edge==='top'?2:0,ws=[{},{},{},{}];ws[index]=w;
        arrange(c,m,ws.map((v,i)=>[v,rs[i]]));
        const before=rs[index],after={...before};c.setEdge(after,edge,c.edgePosition(before,edge)+delta);
        assert.equal(c.adjustRatio(w,before,after),true);
        for(let i=0;i<4;i++) {
            const expected={...rs[i]};
            const moved=edge==='left'||edge==='right'?(i%2?'left':'right'):(i<2?'bottom':'top');
            c.setEdge(expected,moved,c.edgePosition(expected,moved)+delta);
            assert.deepEqual({...c.rects(m).get(c.slotOf(ws[i])[1])},expected);
        }
    }
});
test('a corner resize keeps both connected grid edges aligned',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],right={},below={},diagonal={};c.gap=1;
    const before={x:1,y:1,width:499,height:399};
    arrange(c,m,[[w,before],[right,{x:501,y:1,width:498,height:399}],[below,{x:1,y:401,width:499,height:398}],[diagonal,{x:501,y:401,width:498,height:398}]]);
    assert.equal(c.adjustRatio(w,before,{...before,width:579,height:339}),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(diagonal)[1])},{x:581,y:341,width:418,height:458});
    assert.deepEqual({...c.rects(m).get(c.slotOf(right)[1])},{x:581,y:1,width:418,height:339});
    assert.deepEqual({...c.rects(m).get(c.slotOf(below)[1])},{x:1,y:341,width:579,height:458});
});
test('every connected window contributes its minimum size in either drag direction',()=>{
    for(const grow of [false,true]) {
        const c=backend(),w=window(c),m=c.monitors[0],below={minSize:{width:450,height:100}},right={minSize:{width:300,height:100}};c.gap=1;
        const before={x:1,y:1,width:499,height:399};
        arrange(c,m,[[w,before],[below,{x:1,y:401,width:499,height:398}],[right,{x:501,y:1,width:498,height:798}]]);
        const plan=c.resizeLayout(w,before,{...before,width:grow?950:100});assert.ok(plan);m.root=plan.tree;
        assert.equal(plan.rect.width,grow?697:450);
        assert.equal(c.rects(m).get(c.slotOf(below)[1]).width,plan.rect.width);
        assert.ok(c.rects(m).get(c.slotOf(right)[1]).width>=300);
    }
});
test('an empty break in a shared edge prevents transitive propagation to distant aligned windows',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],near={},far={},farAcross={};c.gap=1;
    const before={x:1,y:1,width:499,height:250},fixed={x:1,y:400,width:499,height:399},across={x:501,y:400,width:498,height:399};
    arrange(c,m,[[w,before],[near,{x:501,y:1,width:498,height:250}],[far,fixed],[farAcross,across]]);
    assert.equal(c.adjustRatio(w,before,{...before,width:599}),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(far)[1])},fixed);
    assert.deepEqual({...c.rects(m).get(c.slotOf(farAcross)[1])},across);
});
test('diagonally touching corners alone do not create a shared resize connection',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],diagonal={};c.gap=1;
    const before={x:1,y:1,width:499,height:399},fixed={x:501,y:401,width:498,height:398};
    arrange(c,m,[[w,before],[diagonal,fixed]]);
    assert.equal(c.adjustRatio(w,before,{...before,width:599}),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(diagonal)[1])},fixed);
});
test('connected neighbors preview live without committing the tree, and Escape restores them',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],below=window(c);c.monitors.pop();c.gap=1;
    const before={x:1,y:1,width:499,height:399},other={x:1,y:401,width:499,height:398};
    arrange(c,m,[[w,before],[below,other]]);w.frameGeometry=before;below.frameGeometry=other;const tree=m.root;
    w.move=false;w.interactiveMoveResizeStarted.emit();w.interactiveMoveResizeStepped.emit({...before,width:576});
    assert.equal(below.frameGeometry.width,576);assert.equal(m.root,tree);assert.equal(c.resizePreviewWindows.size,1);
    w.frameGeometry=before;w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    for(let i=0;i<10;i++)c.placeNextWindow();
    assert.deepEqual({...below.frameGeometry},other);assert.equal(m.root,tree);assert.equal(c.resizePreviewWindows.size,0);
});
test('pausing mid-resize rolls back neighbor previews without changing the saved layout',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],below=window(c);c.monitors.pop();c.gap=1;
    const before={x:1,y:1,width:499,height:399},other={x:1,y:401,width:499,height:398};
    arrange(c,m,[[w,before],[below,other]]);w.frameGeometry=before;below.frameGeometry=other;const tree=m.root;
    c.previewResizeNeighbors(w,before,{...before,width:576});assert.equal(below.frameGeometry.width,576);
    c.setPaused(true);assert.deepEqual({...below.frameGeometry},other);assert.equal(m.root,tree);assert.equal(c.resizePreviewWindows.size,0);
});
test('returning to the original edge during a drag restores previews immediately',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],below=window(c);c.monitors.pop();c.gap=1;
    const before={x:1,y:1,width:499,height:399},other={x:1,y:401,width:499,height:398};
    arrange(c,m,[[w,before],[below,other]]);w.frameGeometry=before;below.frameGeometry=other;
    c.previewResizeNeighbors(w,before,{...before,width:576});assert.equal(below.frameGeometry.width,576);
    c.previewResizeNeighbors(w,before,before);assert.deepEqual({...below.frameGeometry},other);assert.equal(c.resizePreviewWindows.size,0);
});
test('guide removal cannot re-enter placement with the old tree during native resize release',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;c.apply();
    const before={...w.frameGeometry},after={...before,width:before.width-99};
    w.move=false;w.interactiveMoveResizeStarted.emit();w.frameGeometry=after;
    // KWin emits windowRemoved synchronously when an overlay is hidden.
    c.hidePreview=()=>c.apply();const requests=[],place=c.placeWindow;
    c.placeWindow=(w,r)=>{requests.push({...r});place(w,r);};
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
    assert.deepEqual({...w.frameGeometry},after);
    assert.equal(requests.some(r=>r.width===before.width),false,'no stale pre-resize configure');
    assert.equal(c.interactiveWindows.size,0);
});
test('removing an untracked overlay never triggers a retile; removing a managed client does',()=>{
    const c=backend(),w=window(c);let applied=0;c.apply=()=>applied++;
    c.removed({caption:'overlay',normalWindow:false});assert.equal(applied,0);assert.ok(c.slotOf(w));
    c.removed(w);assert.equal(applied,1);assert.equal(c.slotOf(w),null);
});
test('a Wayland Escape frame arriving after Finished cancels the pending tree commit',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],below=window(c);c.monitors.pop();c.gap=1;
    const before={x:1,y:1,width:499,height:399},other={x:1,y:401,width:499,height:398};
    arrange(c,m,[[w,before],[below,other]]);w.frameGeometry=before;below.frameGeometry=other;const tree=m.root;
    w.move=false;w.interactiveMoveResizeStarted.emit();w.interactiveMoveResizeStepped.emit({...before,width:576});
    w.interactiveMoveResizeFinished.emit();
    assert.equal(c.pendingResizeEnds.size,1);assert.equal(c.interactiveWindows.has(w),true);assert.equal(m.root,tree);
    w.frameGeometry=before;c.completeResizeEnds();for(let i=0;i<10;i++)c.placeNextWindow();
    assert.equal(m.root,tree);assert.deepEqual({...below.frameGeometry},other);assert.equal(c.pendingResizeEnds.size,0);
    assert.equal(c.resizeFinishTimer.running,false);assert.equal(c.interactiveWindows.size,0);
});
test('quiescing disconnects every client callback before teardown and is idempotent',()=>{
    const c=backend(),w=window(c);c.apply();
    c.quiesce();c.quiesce();
    assert.equal(c.enabled,false);assert.equal(c.windowConnections.size,0);
    for(const s of ['frameGeometryChanged','interactiveMoveResizeStarted','interactiveMoveResizeStepped','interactiveMoveResizeFinished','minimizedChanged'])assert.equal(w[s].handlers.size,0);
    assert.equal(c.pendingResizeEnds.size,0);assert.equal(c.interactiveWindows.size,0);assert.equal(c.placementQueue.length,0);
    for(const t of ['placementDeadline','placementSpacing','resizeFinishTimer','recoveryTimer','workAreaTimer'])assert.equal(c[t].running,false);
    w.interactiveMoveResizeStarted.emit();assert.equal(c.interactiveWindows.size,0);
});
test('production QML has scoped signal ownership and no destruction-time JavaScript callback',()=>{
    const source=readFileSync(new URL('../src/linux/kwin.qml',`file://${__filename}`),'utf8');
    assert.doesNotMatch(source,/^\s*Component\.onDestruction\s*:/m);
    assert.doesNotMatch(source,/\.connect\(handlers\./);
    assert.match(source,/windowObserver\.createObject\(root,/);
});
test('intentional small vacancies are not swallowed by the legacy five-percent collapse rule',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;
    const before=c.rects(m).get(m.root),after={...before,width:before.width-20};
    assert.equal(c.adjustRatio(w,before,after),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
    assert.equal(c.snapshotTree(m.root).preserveSpace,true);
});
test('quarter half and three-quarter snaps capture within six pixels and release beyond it',()=>{
    for(const axis of ['x','y'])for(const f of [.25,.5,.75])for(const leading of [true,false]) {
        const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;
        const before=c.rects(m).get(m.root),e=axis==='x'?(leading?'left':'right'):(leading?'top':'bottom');
        const p=Math.round(before[axis]+(axis==='x'?before.width:before.height)*f),after={...before};
        c.setEdge(after,e,p+5);
        const hit=c.resizeSnap(w,before,after,[e]);
        assert.equal(c.edgePosition(hit.rect,e),p);assert.equal(hit.guides.length,1);
        c.setEdge(after,e,p+7);
        const free=c.resizeSnap(w,before,after,[e]);
        assert.equal(free.guides.length,0);assert.equal(c.edgePosition(free.rect,e),p+7);
    }
});
test('equal-size and aligned-edge snaps use other visible windows without resizing them',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={};c.gap=1;
    arrange(c,m,[[w,{x:1,y:1,width:998,height:300}],[other,{x:1,y:400,width:333,height:399}]]);
    const before=c.rects(m).get(c.slotOf(w)[1]),after={...before,width:337};
    const hit=c.resizeSnap(w,before,after,['right']);
    assert.equal(hit.rect.width,333);assert.equal(hit.guides.length,1);
    assert.equal(c.rects(m).get(c.slotOf(other)[1]).width,333);
});
test('unachievable snaps do not show misleading guides or violate minimum sizes',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];w.minSize={width:650,height:100};
    const before=c.rects(m).get(m.root),after={...before,width:494};
    assert.equal(c.resizeSnap(w,before,after,['right']).guides.length,0);
    assert.equal(c.adjustRatio(w,before,after),false);
    assert.deepEqual({...c.rects(m).get(m.root)},{...before});
});
test('interactive resize snaps gently, releases with pointer movement, and Escape preserves the layout',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;c.apply();w.move=false;
    const before={...w.frameGeometry};c.Workspace.cursorPos={x:c.rectRight(before),y:100};
    w.interactiveMoveResizeStarted.emit();
    c.Workspace.cursorPos={x:505,y:100};
    w.frameGeometry={...before,width:504};w.interactiveMoveResizeStepped.emit(w.frameGeometry);
    assert.equal(c.rectRight(w.frameGeometry),500);assert.equal(c.resizeGuide.visible,true);
    c.Workspace.cursorPos={x:510,y:100};w.frameGeometry={...before,width:509};w.interactiveMoveResizeStepped.emit(w.frameGeometry);
    assert.equal(c.resizeGuide.visible,false);assert.equal(c.rectRight(w.frameGeometry),510);
    w.frameGeometry=before;w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},before);assert.equal(c.resizeGuide.visible,false);
});
test('a delayed first Wayland frame does not cancel the resize and corners can acquire a second edge',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;c.apply();w.move=false;
    const initial={...w.frameGeometry};c.adjustRatio(w,initial,{...initial,width:600,height:600});c.apply();
    const before={...w.frameGeometry};c.Workspace.cursorPos={x:c.rectRight(before),y:c.rectBottom(before)};
    w.interactiveMoveResizeStarted.emit();
    c.Workspace.cursorPos={x:c.rectRight(before)+30,y:c.rectBottom(before)};
    w.interactiveMoveResizeStepped.emit(before);
    assert.equal(w.frameGeometry.width,before.width+30);
    c.Workspace.cursorPos={x:c.rectRight(before)+30,y:c.rectBottom(before)+30};
    w.interactiveMoveResizeStepped.emit(w.frameGeometry);
    assert.equal(w.frameGeometry.height,before.height+30);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.equal(c.rects(m).get(c.slotOf(w)[1]).height,before.height+30);
});
test('restarting adopts valid actual frames instead of replaying a stale default tree',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={frameGeometry:{x:600,y:10,width:390,height:780}};
    w.frameGeometry={x:10,y:10,width:350,height:500};
    c.splitSlot(m,m.root,'x',false,other);
    c.adoptExistingGeometry();
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},{...w.frameGeometry});
    assert.deepEqual({...c.rects(m).get(c.slotOf(other)[1])},{...other.frameGeometry});
});
test('snapshot round trip keeps deliberately small resized vacancies exact',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;
    w.internalId='a';const before=c.rects(m).get(m.root),after={...before,width:before.width-20};
    c.adjustRatio(w,before,after);
    const saved={gap:1,windows:[{token:'a',app:'test',title:'test',rect:after}],monitors:[{root:c.snapshotTree(m.root)}]};
    c.Workspace.stackingOrder=[w];c.syncMonitors=()=>{};c.snapshotRestored={call(){}};
    c.restoreSnapshot(JSON.stringify(saved));
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
});
test('collision-limited resizing advances to a valid local edge instead of undoing the gesture',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={};c.gap=1;
    c.splitSlot(m,m.root,'x',false,other);
    const before=c.rects(m).get(c.slotOf(w)[1]),neighbor=c.rects(m).get(c.slotOf(other)[1]);
    arrange(c,m,[[w,{...before,width:before.width-100}],[other,{...neighbor}]]);
    const start=c.rects(m).get(c.slotOf(w)[1]),raw={...start,width:start.width+250};
    const bounded=c.constrainedResize(w,start,raw);
    assert.ok(bounded.width>start.width+90);assert.ok(c.rectRight(bounded)+1<=neighbor.x);
    assert.equal(c.adjustRatio(w,start,bounded),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(other)[1])},{...neighbor});
});
test('occupied minimum sizes and ordinary large empty slots remain intact',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],other={minSize:{width:200,height:200}};
    c.splitSlot(m,m.root,'x',false,other);m.root.ratio=.95;
    assert.ok(c.rects(m).get(c.slotOf(other)[1]).width>=200);
    other.minimized=true;m.root.ratio=.5;
    assert.ok(c.rects(m).get(c.slotOf(other)[1]).width>0);
    m.root.ratio=.95;
    assert.equal(c.area(c.rects(m).get(c.slotOf(other)[1])),0);
    other.minimized=false;
    assert.ok(c.rects(m).get(c.slotOf(other)[1]).width>=200);
});
test('collapsed placeholders do not add invisible minimum-size gaps to ancestors',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],hidden={minimized:true};w.minSize={width:300,height:200};
    c.splitSlot(m,m.root,'y',false,hidden);m.root.ratio=.95;
    assert.deepEqual({...c.minimumSize(m.root)},{width:300,height:200});
});
test('panel work-area changes reserve space on all four edges without a monitor event',()=>{
    for(const available of [
        {x:0,y:30,width:1000,height:770},{x:0,y:0,width:1000,height:770},
        {x:30,y:0,width:970,height:800},{x:0,y:0,width:970,height:800},
    ]) {
        const c=backend(),w=window(c),m=c.monitors[0];c.gap=1;c.apply();
        const originalRoot=m.root;c.Workspace.clientArea=()=>available;c.workAreasChanged();
        assert.equal(m.root,originalRoot);
        assert.deepEqual({...w.frameGeometry},{...c.inset(available)});
        let calls=0;c.apply=()=>calls++;c.workAreasChanged();assert.equal(calls,0);
    }
});
test('panel changes do not interrupt a drag and are applied on completion',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.apply();
    const before={...w.frameGeometry};w.interactiveMoveResizeStarted.emit();
    c.Workspace.clientArea=()=>({x:0,y:0,width:1000,height:760});
    c.workAreasChanged();assert.deepEqual({...w.frameGeometry},before);assert.equal(m.area.height,800);
    w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();assert.equal(m.area.height,760);
    assert.equal(w.frameGeometry.height,740);
});
test('work-area refresh preserves other monitors and restores space when a panel leaves',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];
    const second={output:2,area:{x:1000,y:0,width:1000,height:800},root:c.leaf()};c.monitors.push(second);
    let panel=true;c.Workspace.clientArea=(_option,output)=>output===2?second.area:
        {x:0,y:0,width:1000,height:panel?770:800};
    c.workAreasChanged();assert.equal(w.frameGeometry.height,750);assert.equal(second.area.height,800);
    panel=false;c.workAreasChanged();assert.equal(w.frameGeometry.height,780);
});
test('pause stops placements and previews; resume retains the same layout',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];c.apply();const tree=m.root;
    c.setPaused(true);w.move=true;w.interactiveMoveResizeStarted.emit();
    w.frameGeometry={x:200,y:200,width:100,height:100};w.interactiveMoveResizeFinished.emit();c.completeResizeEnds();
    assert.equal(w.frameGeometry.x,200);assert.equal(c.dragPreview.visible,false);
    c.setPaused(false);assert.equal(m.root,tree);assert.equal(w.frameGeometry.x,10);
});
test('layout reconciliation never activates an unrelated stack',()=>{
    const c=backend(),w=window(c),other={...w,desktopFileName:'other'};
    c.assign(c.slotOf(w)[1],other);c.focused=w;c.Workspace.activeWindow=w;
    c.currentPlacement={window:other,active:true};c.finishCurrentPlacement();
    assert.equal(c.Workspace.activeWindow,w);
});
test('hidden stack members are not raised or selected by cycling',()=>{
    const c=backend(),w=window(c),s=c.slotOf(w)[1],hidden={...w,minimized:true};
    c.assign(s,hidden);c.focused=w;c.Workspace.activeWindow=w;
    assert.equal(c.placements().some(p=>p.active),false);c.cycle(1);
    assert.equal(c.Workspace.activeWindow,w);
    const other={...w,desktopFileName:'other',frameGeometry:{...w.frameGeometry}};c.assign(s,other);
    c.cycle(1);assert.equal(c.Workspace.activeWindow,other);c.cycle(1);assert.equal(c.Workspace.activeWindow,w);
});
test('snapshot restores saved geometry and leaves extra windows floating',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];m.name='screen';w.internalId='a';
    const extra={...w,internalId:'b',desktopFileName:'extra',caption:'extra'};
    c.Workspace.stackingOrder=[w,extra];c.syncMonitors=()=>{};c.snapshotRestored={call(){}};
    const saved={schema:1,gap:1,windows:[{token:'old',app:'test',title:'test',rect:{x:1,y:1,width:998,height:798},floating:false}],monitors:[{name:'screen',area:m.area,root:{kind:'leaf',windows:['old'],active:0}}]};
    c.restoreSnapshot(JSON.stringify(saved));
    assert.equal(c.gap,1);assert.equal(c.slotOf(w)[1],m.root);
    assert.equal(c.floating.has(extra),true);assert.equal(c.slotOf(extra),null);
    assert.deepEqual({...w.frameGeometry},{x:1,y:1,width:998,height:798});
});
test('an app launched after snapshot loading claims its saved slot',()=>{
    const c=backend(),w=window(c),m=c.monitors[0];m.name='screen';w.internalId='a';
    c.Workspace.stackingOrder=[w];c.syncMonitors=()=>{};c.snapshotRestored={call(){}};
    const saved={schema:1,gap:1,windows:[{token:'later',app:'later',title:'later',rect:{x:1,y:1,width:998,height:798},floating:false}],monitors:[{name:'screen',area:m.area,root:{kind:'leaf',windows:['later'],active:0}}]};
    c.restoreSnapshot(JSON.stringify(saved));
    const later={...w,internalId:'new',desktopFileName:'later',caption:'later'};
    assert.equal(c.appeared(later),true);assert.equal(c.slotOf(later)[1],m.root);
    assert.equal(c.floating.has(w),true);
    assert.equal(JSON.parse(c.snapshotRestored.arguments[0]).missing,0);
});
