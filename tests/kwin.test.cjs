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
        previewOwner:null,previewGeometry:null,dragPreview:{visible:false},resizeGuide:{visible:false},resizeGuides:[],
        placementDeadline:timer(), placementSpacing:timer(), recoveryTimer:timer(),workAreaTimer:timer(),
        KWin:{MaximizeArea:0},
        Workspace:{currentDesktop:1,currentActivity:'test',raiseWindow(){},hideOutline(){}},
    });
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
    w.interactiveMoveResizeFinished.emit();assert.equal(c.interactiveWindows.size,0);
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
    w.interactiveMoveResizeFinished.emit();
    assert.equal(dropped,false);assert.equal(c.interactiveWindows.size,0);
});
test('dragging an excluded prompt never enrolls it in the tiling tree',()=>{
    const c=backend();const w=window(c);c.detach(w);w.desktopFileName='org.kde.kwin.eisprompter';w.move=true;
    let dropped=false;c.drop=()=>{dropped=true;};
    w.interactiveMoveResizeStarted.emit();w.frameGeometry={x:200,y:200,width:300,height:200};
    w.interactiveMoveResizeFinished.emit();
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
    w.interactiveMoveResizeFinished.emit();assert.equal(c.dragPreview.visible,false);
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
test('undersized quarters fall back to a fitting half or full area',()=>{
    const c=backend(),w=window(c);w.minSize={width:400,height:300};
    const r={x:0,y:0,width:600,height:800};
    assert.equal(c.fittingEmptyZone(w,r,'top-left'),'top');
    w.minSize={width:400,height:600};assert.equal(c.fittingEmptyZone(w,r,'top-left'),'center');
    w.minSize={width:700,height:600};assert.equal(c.fittingEmptyZone(w,r,'top-left'),'unavailable');
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
test('a shared resize affects only directly touching neighbors, not their unrelated siblings',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],neighbor={},unrelated={};c.gap=1;
    c.splitSlot(m,m.root,'x',false,neighbor);c.splitSlot(m,c.slotOf(w)[1],'y',false,unrelated);
    const old=c.rects(m),before=old.get(c.slotOf(w)[1]),fixed=old.get(c.slotOf(unrelated)[1]);
    const after={...before,width:before.width+100};
    assert.equal(c.adjustRatio(w,before,after),true);
    const next=c.rects(m);
    assert.deepEqual({...next.get(c.slotOf(unrelated)[1])},{...fixed});
    assert.deepEqual({...next.get(c.slotOf(w)[1])},after);
    assert.equal(next.get(c.slotOf(neighbor)[1]).x,c.rectRight(after)+1);
});
test('shrinking a monitor-filling window creates reusable space without a parent split',()=>{
    const c=backend(),w=window(c),m=c.monitors[0],before=c.rects(m).get(m.root);
    const after={...before,width:before.width-210};
    assert.equal(c.adjustRatio(w,before,after),true);
    assert.deepEqual({...c.rects(m).get(c.slotOf(w)[1])},after);
    assert.ok(c.leaves(m.root).some(s=>!s.windows.length));
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
    c.splitSlot(m,m.root,'y',false,other);
    const otherBefore=c.rects(m).get(c.slotOf(other)[1]);
    c.adjustRatio(other,otherBefore,{...otherBefore,width:333});
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
    w.frameGeometry=before;w.interactiveMoveResizeFinished.emit();
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
    w.interactiveMoveResizeFinished.emit();
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
    c.adjustRatio(w,before,{...before,width:before.width-100});
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
    w.interactiveMoveResizeFinished.emit();assert.equal(m.area.height,760);
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
    w.frameGeometry={x:200,y:200,width:100,height:100};w.interactiveMoveResizeFinished.emit();
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
