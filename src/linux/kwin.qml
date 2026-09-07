import QtQuick
import QtQuick.Window
import QtQml.Models
import org.kde.kwin
import org.kde.ksvg as KSvg

Item {
    id: root

    property int gap: __TILEKEEP_GAP__
    property bool paused: false
    readonly property bool dryRun: __TILEKEEP_DRY_RUN__
    readonly property real minRatio: 0.05
    readonly property real maxRatio: 0.95
    property var monitors: []
    property var focused: null
    property var floating: new Set()
    property var identities: new Map()
    property var expectedGeometry: new Map()
    property var deferredPlacements: new Map()
    property var interactiveWindows: new Set()
    property var placementQueue: []
    property var currentPlacement: null
    property int placementAttempt: 0
    property var windowConnections: new Map()
    property var previewOwner: null
    property var previewGeometry: null
    property int sequence: 1
    property var pendingSnapshot: null

    function rect(r) { return {x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height)}; }
    function rectRight(r) { return r.x + r.width; }
    function rectBottom(r) { return r.y + r.height; }
    function area(r) { return r.width * r.height; }
    function contains(r, p) { return p.x >= r.x && p.y >= r.y && p.x < rectRight(r) && p.y < rectBottom(r); }
    function inset(r) { return {x:r.x+gap, y:r.y+gap, width:Math.max(0,r.width-2*gap), height:Math.max(0,r.height-2*gap)}; }
    function leaf() { return {kind:"leaf", windows:[], active:0, vacated:null, remembered:null, parent:null}; }
    function leaves(node, out) {
        out = out || [];
        if (node.kind === "leaf") out.push(node);
        else { leaves(node.first, out); leaves(node.second, out); }
        return out;
    }
    function allWindows(node) {
        const out=[];
        for (const slot of leaves(node)) for (const window of slot.windows) out.push(window);
        return out;
    }
    function slotOf(w) {
        for (const m of monitors) for (const s of leaves(m.root)) if (s.windows.includes(w)) return [m,s];
        return null;
    }
    function replaceNode(old, replacement, monitor) {
        const parent = old.parent;
        replacement.parent = parent;
        if (!parent) monitor.root = replacement;
        else if (parent.first === old) parent.first = replacement;
        else parent.second = replacement;
    }
    function splitSlot(monitor, target, axis, newFirst, w) {
        const added = leaf(); added.windows = [w];
        const available=rects(monitor).get(target), a=minimumSize(target), b=minimumSize(added);
        const needed=axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
        // Do not create an impossible split and hope the client ignores its
        // minimum. Sharing the existing slot keeps both windows usable.
        if(needed.width>available.width||needed.height>available.height) { assign(target,w);return target; }
        const split = {kind:"split", axis, ratio:0.5, first:newFirst?added:target, second:newFirst?target:added, parent:null};
        const oldParent = target.parent;
        split.parent = oldParent; target.parent = split; added.parent = split;
        if (!oldParent) monitor.root = split;
        else if (oldParent.first === target) oldParent.first = split;
        else oldParent.second = split;
        return added;
    }
    function minimumSize(node,exclude) {
        if(node.kind==="leaf") {
            let width=0,height=0;
            for(const w of node.windows) {
                // Invisible windows keep their remembered slot, but must not
                // prevent a visible neighbor from using the free space.
                if(w===exclude||!visibleHere(w))continue;
                const min=w.minSize||{width:0,height:0},frame=w.frameGeometry,client=w.clientGeometry;
                const borderWidth=frame&&client?Math.max(0,frame.width-client.width):0;
                const borderHeight=frame&&client?Math.max(0,frame.height-client.height):0;
                width=Math.max(width,Math.ceil(min.width+borderWidth));
                height=Math.max(height,Math.ceil(min.height+borderHeight));
            }
            return {width,height};
        }
        const collapsed=collapsedSide(node);
        if(collapsed)return minimumSize(collapsed<0?node.second:node.first,exclude);
        if(vacantSubtree(node))return {width:0,height:0};
        const a=minimumSize(node.first,exclude),b=minimumSize(node.second,exclude);
        return node.axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
    }
    function splitExtent(usable,ratio,firstMin,secondMin) {
        const preferred=Math.round(usable*ratio);
        return firstMin+secondMin<=usable?Math.max(firstMin,Math.min(usable-secondMin,preferred)):preferred;
    }
    function vacantSubtree(node) { return leaves(node).every(s=>vacantHere(s)); }
    function collapsedSide(node) {
        const first=vacantSubtree(node.first),second=vacantSubtree(node.second);
        if(first&&!second&&node.ratio<=minRatio)return -1;
        if(second&&!first&&node.ratio>=maxRatio)return 1;
        return 0;
    }
    function compute(node, r, result) {
        // Keep internal rectangles too: resizing a nested split needs its bounds.
        result.set(node, r);
        if (node.kind === "leaf") return;
        // A resize against the old 5%/95% stop can leave a thin, unusable
        // vacancy. Give that space (and its extra gap) to the visible sibling.
        // Keep the leaf and its occupants so restoring a hidden window or
        // opening a replacement still has a remembered place in the tree.
        const collapsed=collapsedSide(node),collapseFirst=collapsed<0,collapseSecond=collapsed>0;
        if(collapseFirst||collapseSecond) {
            const empty=node.axis==="x"?
                {x:collapseFirst?r.x:rectRight(r),y:r.y,width:0,height:r.height}:
                {x:r.x,y:collapseFirst?r.y:rectBottom(r),width:r.width,height:0};
            compute(node.first,collapseFirst?empty:r,result);
            compute(node.second,collapseSecond?empty:r,result);
            return;
        }
        const a=minimumSize(node.first),b=minimumSize(node.second);
        if (node.axis === "x") {
            const usable = Math.max(0, r.width-gap), first = splitExtent(usable,node.ratio,a.width,b.width);
            compute(node.first,{x:r.x,y:r.y,width:first,height:r.height},result);
            compute(node.second,{x:r.x+first+gap,y:r.y,width:usable-first,height:r.height},result);
        } else {
            const usable = Math.max(0, r.height-gap), first = splitExtent(usable,node.ratio,a.height,b.height);
            compute(node.first,{x:r.x,y:r.y,width:r.width,height:first},result);
            compute(node.second,{x:r.x,y:r.y+first+gap,width:r.width,height:usable-first},result);
        }
    }
    function rects(m) { const out=new Map(); compute(m.root,inset(m.area),out); return out; }
    function windowRect(w) { return rect(w.frameGeometry); }
    function identity(w) { return String(w.desktopFileName || w.resourceClass || w.resourceName || "").toLowerCase(); }
    function tileable(w) {
        // The Xwayland remote-control permission UI is a normal window, not a
        // transient dialog. Keep compositor security prompts outside the layout.
        if(w&&identity(w)==="org.kde.kwin.eisprompter")return false;
        return w && w.managed && !w.deleted && w.normalWindow && !w.specialWindow && !w.transient &&
            w.moveable && w.resizeable && !w.skipTaskbar && String(w.caption || "").length > 0;
    }
    function visibleHere(w) {
        if (w.minimized || w.fullScreen || w.hidden || w.maximizeMode) return false;
        if (!w.onAllDesktops && w.desktops && !w.desktops.includes(Workspace.currentDesktop)) return false;
        if (w.activities && w.activities.length && !w.activities.includes(Workspace.currentActivity)) return false;
        return true;
    }
    function monitorForOutput(output) { return monitors.find(m => m.output === output) || monitors[0]; }
    function refreshWorkAreas() {
        let changed=false;
        for(const m of monitors) {
            const a=rect(Workspace.clientArea(KWin.MaximizeArea,m.output,Workspace.currentDesktop)),old=m.area;
            if(a.x!==old.x||a.y!==old.y||a.width!==old.width||a.height!==old.height) {
                m.area=a;changed=true;
            }
        }
        return changed;
    }
    function workAreasChanged() {
        if(!enabled||interactiveWindows.size)return;
        if(refreshWorkAreas())apply();
    }
    function syncMonitors() {
        const old = monitors, next=[];
        for (const output of Workspace.screens) {
            const existing=old.find(m=>m.name===output.name);
            const a=rect(Workspace.clientArea(KWin.MaximizeArea,output,Workspace.currentDesktop));
            next.push(existing ? Object.assign(existing,{output,area:a}) : {name:output.name,output,area:a,root:leaf()});
        }
        const gone=[];
        for (const m of old) if (!next.includes(m)) for (const w of allWindows(m.root)) gone.push(w);
        monitors=next;
        if (monitors.length) for (const w of gone) appeared(w);
    }
    function assign(slot,w) { slot.windows.push(w); slot.active=slot.windows.length-1; slot.vacated=null; slot.remembered=null; }
    function appeared(w) {
        if(pendingSnapshot&&restorePendingWindow(w))return true;
        if (!tileable(w) || slotOf(w) || floating.has(w)) return false;
        const m=monitorForOutput(w.output); if (!m) return false;
        const id=identity(w); identities.set(w,id);
        const ls=leaves(m.root), remembered=ls.filter(s=>!s.windows.length&&s.remembered===id).sort((a,b)=>(b.vacated||0)-(a.vacated||0))[0];
        if (remembered) assign(remembered,w);
        else {
            const empty=ls.filter(s=>!s.windows.length).sort((a,b)=>(b.vacated||0)-(a.vacated||0))[0];
            if (empty) assign(empty,w);
            else {
                const map=rects(m), focus=focused&&slotOf(focused), target=focus&&focus[0]===m?focus[1]:ls.sort((a,b)=>area(map.get(b))-area(map.get(a)))[0];
                const r=map.get(target); splitSlot(m,target,r.width>=r.height?"x":"y",false,w);
            }
        }
        console.log("Tilekeep: tracking",id,String(w.caption));
        return true;
    }
    function detach(w) {
        const found=slotOf(w); if (!found) return;
        const s=found[1], i=s.windows.indexOf(w); s.windows.splice(i,1);
        if (!s.windows.length) { s.active=0; s.vacated=sequence++; s.remembered=identities.get(w)||null; }
        else { if(i<s.active)s.active--; s.active=Math.min(s.active,s.windows.length-1); }
    }
    function vanished(w) {
        hidePreview(w);
        detach(w);floating.delete(w);identities.delete(w);expectedGeometry.delete(w);
        deferredPlacements.delete(w);interactiveWindows.delete(w);
        placementQueue=placementQueue.filter(p=>p.window!==w);
        if(currentPlacement&&currentPlacement.window===w)finishCurrentPlacement();
        if(focused===w)focused=null;
    }
    function placements() {
        const out=[];
        for (const m of monitors) {
            const map=rects(m);
            for (const s of leaves(m.root)) {
                const stack=s.windows.length>1;
                for(let i=0;i<s.windows.length;i++) out.push({window:s.windows[i],rect:map.get(s),output:m.output,active:stack&&i===s.active});
            }
        }
        return out.sort((a,b)=>Number(a.active)-Number(b.active));
    }
    function placeWindow(w,r) {
        if(w.tile) w.tile.unmanage(w);
        w.setMaximize(false,false);
        const next=Object.assign({},w.frameGeometry);
        next.x=r.x;next.y=r.y;next.width=r.width;next.height=r.height;
        w.frameGeometry=next;
    }
    function apply() {
        if (!enabled || paused || interactiveWindows.size) return;
        refreshWorkAreas();
        const next=placements();
        placementDeadline.stop();placementSpacing.stop();
        placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
        for(const p of next) {
            if (!visibleHere(p.window)) continue;
            if (dryRun) console.log("Tilekeep dry-run:",identity(p.window),JSON.stringify(p.rect));
            else {
                console.log("Tilekeep placement:",identity(p.window),String(p.window.caption),JSON.stringify(p.rect),"output",p.output.name);
                placementQueue.push(p);
            }
        }
        if(!dryRun) placeNextWindow();
    }
    function placeNextWindow() {
        if(!enabled||currentPlacement||!placementQueue.length)return;
        currentPlacement=placementQueue.shift();placementAttempt=0;
        const p=currentPlacement;
        if(!tileable(p.window)||!visibleHere(p.window)||floating.has(p.window)) { finishCurrentPlacement();return; }
        if(p.window.output===p.output&&geometryMatches(windowRect(p.window),p.rect)) { finishCurrentPlacement();return; }
        if(p.window.output!==p.output)Workspace.sendClientToScreen(p.window,p.output);
        expectedGeometry.set(p.window,p.rect);
        // A move can complete synchronously; arm the timer before sending it.
        placementDeadline.restart();
        placeWindow(p.window,p.rect);
    }
    function finishCurrentPlacement() {
        const p=currentPlacement;if(!p)return;
        placementDeadline.stop();
        expectedGeometry.delete(p.window);
        if(p.active&&tileable(p.window)&&visibleHere(p.window)) { Workspace.raiseWindow(p.window);Workspace.activeWindow=p.window; }
        currentPlacement=null;
        placementSpacing.restart();
    }
    function geometryMatches(got,wanted) {
        return wanted&&Math.abs(got.x-wanted.x)<=2&&Math.abs(got.y-wanted.y)<=2&&Math.abs(got.width-wanted.width)<=2&&Math.abs(got.height-wanted.height)<=2;
    }
    function retryDeferredPlacements() {
        if(!enabled||dryRun||interactiveWindows.size||currentPlacement||placementQueue.length)return;
        for(const [w,p] of deferredPlacements) {
            if(tileable(w)&&visibleHere(w)&&!floating.has(w)&&slotOf(w)&&!geometryMatches(windowRect(w),p.rect))placementQueue.push(p);
        }
        deferredPlacements.clear();
        placeNextWindow();
    }
    function placementTimedOut() {
        const p=currentPlacement;if(!p)return;
        if(!tileable(p.window)||!visibleHere(p.window)||floating.has(p.window)) { finishCurrentPlacement();return; }
        const got=windowRect(p.window);
        const wanted=expectedGeometry.get(p.window);
        if(!wanted||geometryMatches(got,wanted)) {
            finishCurrentPlacement();return;
        }
        if(placementAttempt<2) {
            placementAttempt++;placementDeadline.restart();placeWindow(p.window,wanted);return;
        }
        // Wayland clients can stop drawing while an output sleeps. A timeout is
        // not a refusal: retain the request and reconcile after repaint resumes.
        deferredPlacements.set(p.window,p);
        finishCurrentPlacement();
    }
    function distance(r,p) {
        const dx=p.x<r.x?r.x-p.x:p.x>=rectRight(r)?p.x-rectRight(r)+1:0;
        const dy=p.y<r.y?r.y-p.y:p.y>=rectBottom(r)?p.y-rectBottom(r)+1:0;
        return dx*dx+dy*dy;
    }
    function slotAt(p) {
        const m=monitors.find(m=>contains(m.area,p)); if(!m)return null;
        const map=rects(m), ls=leaves(m.root).filter(s=>area(map.get(s))>0), s=ls.find(s=>contains(map.get(s),p))||ls.sort((a,b)=>distance(map.get(a),p)-distance(map.get(b),p))[0];
        if(!s)return null;
        return [m,s,map.get(s)];
    }
    function zone(r,p) {
        const x=(p.x-r.x)/r.width,y=(p.y-r.y)/r.height;
        if(x>=.25&&x<=.75&&y>=.25&&y<=.75)return "center";
        return Math.abs(x-.5)>=Math.abs(y-.5)?(x<.5?"left":"right"):(y<.5?"top":"bottom");
    }
    function vacantHere(slot) { return !slot.windows.some(w=>visibleHere(w)); }
    function dropPreview(w,hit,z) {
        const target=hit[1],r=hit[2],source=slotOf(w);
        if(z==="center"||vacantHere(target)||(source&&source[1]===target&&target.windows.length<2))return r;
        const axis=z==="left"||z==="right"?"x":"y",newFirst=z==="left"||z==="top";
        const added=leaf();added.windows=[w];
        const a=minimumSize(target,w),b=minimumSize(added);
        const needed=axis==="x"?{width:a.width+b.width+gap,height:Math.max(a.height,b.height)}:
            {width:Math.max(a.width,b.width),height:a.height+b.height+gap};
        if(needed.width>r.width||needed.height>r.height)return r;
        const usable=(axis==="x"?r.width:r.height)-gap;
        const first=splitExtent(usable,.5,newFirst?b[axis==="x"?"width":"height"]:a[axis==="x"?"width":"height"],newFirst?a[axis==="x"?"width":"height"]:b[axis==="x"?"width":"height"]);
        const offset=newFirst?0:first+gap,extent=newFirst?first:usable-first;
        return axis==="x"?{x:r.x+offset,y:r.y,width:extent,height:r.height}:{x:r.x,y:r.y+offset,width:r.width,height:extent};
    }
    function drop(w,p,stack) {
        if(floating.has(w))return false;
        const hit=slotAt(p); if(!hit)return false;
        const [m,target,r]=hit,source=slotOf(w);
        // A minimized/off-desktop window is not an occupied drop target. Use
        // the whole slot even at its edges, swapping its remembered occupants
        // back to the source so restoring them does not cover the dropped one.
        const z=vacantHere(target)?"center":zone(r,p);
        if(source&&source[1]===target) {
            if(z==="center"||target.windows.length<2)return false;
            detach(w); splitSlot(m,target,z==="left"||z==="right"?"x":"y",z==="left"||z==="top",w); return true;
        }
        if(!target.windows.length) { detach(w); assign(target,w); return true; }
        if(z==="center"&&stack) { detach(w); assign(target,w); return true; }
        if(z==="center") {
            if(!source) { splitSlot(m,target,r.width>=r.height?"x":"y",false,w); return true; }
            const a=source[1],temp={windows:a.windows,active:a.active,vacated:a.vacated,remembered:a.remembered};
            a.windows=target.windows;a.active=target.active;a.vacated=target.vacated;a.remembered=target.remembered;
            target.windows=temp.windows;target.active=temp.active;target.vacated=temp.vacated;target.remembered=temp.remembered; return true;
        }
        detach(w); splitSlot(m,target,z==="left"||z==="right"?"x":"y",z==="left"||z==="top",w); return true;
    }
    function adjustRatio(w,before,after) {
        const found=slotOf(w); if(!found)return;
        const m=found[0],s=found[1];
        const changes=[
            ["x",before.x,after.x,true],["x",rectRight(before),rectRight(after),false],
            ["y",before.y,after.y,true],["y",rectBottom(before),rectBottom(after),false]
        ].filter(v=>Math.abs(v[1]-v[2])>2);
        for(const [axis,oldPos,newPos,isStart] of changes) {
            let child=s,parent=s.parent;
            while(parent) {
                if(parent.axis===axis&&((isStart&&parent.second===child)||(!isStart&&parent.first===child))) {
                    const bounds=rects(m).get(parent);
                    const start=axis==="x"?bounds.x:bounds.y,total=axis==="x"?bounds.width:bounds.height;
                    const lower=vacantSubtree(parent.first)?0:minRatio;
                    const upper=vacantSubtree(parent.second)?1:maxRatio;
                    parent.ratio=Math.max(lower,Math.min(upper,(newPos-start-gap*(isStart?1:0))/Math.max(1,total-gap))); break;
                }
                child=parent;parent=parent.parent;
            }
        }
    }
    function subtreeEmpty(node) { return leaves(node).every(s=>!s.windows.length); }
    function compactNode(node) {
        if(node.kind==="leaf")return node;
        node.first=compactNode(node.first);node.first.parent=node;
        node.second=compactNode(node.second);node.second.parent=node;
        if(subtreeEmpty(node.first))return node.second;
        if(subtreeEmpty(node.second))return node.first;
        return node;
    }
    function compact(m) { m.root=compactNode(m.root);m.root.parent=null; }
    function cycle(delta) {
        if(paused)return;
        const f=focused&&slotOf(focused);if(!f||f[1].windows.length<2)return;
        const s=f[1];s.active=(s.active+delta+s.windows.length)%s.windows.length;focused=s.windows[s.active];apply();
    }
    function showPreview(w,r) {
        if(!enabled||paused||dryRun||!tileable(w)||floating.has(w))return;
        previewOwner=w;
        const old=previewGeometry;
        if(!old||old.x!==r.x||old.y!==r.y||old.width!==r.width||old.height!==r.height) {
            previewGeometry=rect(r);
            dragPreview.x=r.x;dragPreview.y=r.y;
            dragPreview.width=r.width;dragPreview.height=r.height;
        }
        if(!dragPreview.visible)dragPreview.visible=true;
    }
    function hidePreview(w) {
        if(w&&previewOwner!==w)return;
        dragPreview.visible=false;previewOwner=null;previewGeometry=null;
    }
    function connectWindow(w) {
        if(windowConnections.has(w))return;
        let drag=null;
        const handlers={};
        handlers.geometry=()=>{
            const wanted=expectedGeometry.get(w);if(!wanted)return;
            const got=windowRect(w);
            if(currentPlacement&&currentPlacement.window===w&&geometryMatches(got,wanted)) {
                placementDeadline.stop();finishCurrentPlacement();
            }
        };
        handlers.started=()=>{
            hidePreview();
            interactiveWindows.add(w);
            placementDeadline.stop();placementSpacing.stop();
            placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
            if(!paused&&tileable(w)&&!floating.has(w))drag={rect:windowRect(w),moving:w.move};
        };
        handlers.stepped=g=>{
            if(!drag||!drag.moving)return;
            const hit=slotAt(Workspace.cursorPos);if(!hit){hidePreview(w);return;}
            const r=dropPreview(w,hit,zone(hit[2],Workspace.cursorPos));
            showPreview(w,r);
        };
        handlers.finished=()=>{
            interactiveWindows.delete(w);
            hidePreview(w);if(!drag||paused){drag=null;apply();return;}
            const d=drag;drag=null;
            // KWin restores the starting geometry before emitting Finished when
            // Escape cancels a drag. The cursor can still be over another slot.
            if(d.moving) {
                if(!geometryMatches(windowRect(w),d.rect))drop(w,Workspace.cursorPos,false);
            } else adjustRatio(w,d.rect,windowRect(w));
            apply();
        };
        handlers.minimized=()=>{apply();};
        w.frameGeometryChanged.connect(handlers.geometry);
        w.interactiveMoveResizeStarted.connect(handlers.started);
        w.interactiveMoveResizeStepped.connect(handlers.stepped);
        w.interactiveMoveResizeFinished.connect(handlers.finished);
        w.minimizedChanged.connect(handlers.minimized);
        w.maximizedChanged.connect(handlers.minimized);
        w.fullScreenChanged.connect(handlers.minimized);
        w.desktopsChanged.connect(handlers.minimized);
        w.activitiesChanged.connect(handlers.minimized);
        windowConnections.set(w,handlers);
    }
    function disconnectWindow(w) {
        const h=windowConnections.get(w);if(!h)return;
        try { w.frameGeometryChanged.disconnect(h.geometry); } catch(e) {}
        try { w.interactiveMoveResizeStarted.disconnect(h.started); } catch(e) {}
        try { w.interactiveMoveResizeStepped.disconnect(h.stepped); } catch(e) {}
        try { w.interactiveMoveResizeFinished.disconnect(h.finished); } catch(e) {}
        try { w.minimizedChanged.disconnect(h.minimized); } catch(e) {}
        try { w.maximizedChanged.disconnect(h.minimized); } catch(e) {}
        try { w.fullScreenChanged.disconnect(h.minimized); } catch(e) {}
        try { w.desktopsChanged.disconnect(h.minimized); } catch(e) {}
        try { w.activitiesChanged.disconnect(h.minimized); } catch(e) {}
        windowConnections.delete(w);
    }
    function stackAtCursor() {
        if(paused)return;
        const w=Workspace.activeWindow,hit=slotAt(Workspace.cursorPos);if(!tileable(w)||floating.has(w)||!hit||hit[1].windows.includes(w))return;
        detach(w);assign(hit[1],w);focused=w;apply();
    }
    function stop() {
        enabled=false;placementDeadline.stop();placementSpacing.stop();recoveryTimer.stop();workAreaTimer.stop();hidePreview();
        unloadCall.service="org.kde.KWin";
        unloadCall.path="/Scripting";
        unloadCall.method="unloadScript";
        unloadCall.arguments=["tilekeep-runtime"];
        unloadCall.call();
    }
    function setPaused(value) {
        paused=value;hidePreview();placementDeadline.stop();placementSpacing.stop();
        placementQueue=[];currentPlacement=null;expectedGeometry.clear();deferredPlacements.clear();
        if(!paused)apply();
    }
    function snapshotWindows() {
        return Workspace.stackingOrder.filter(w=>tileable(w)).map(w=>({token:String(w.internalId),app:identity(w),title:String(w.caption),pid:w.pid,rect:windowRect(w),floating:floating.has(w)}));
    }
    function snapshotTree(node) {
        if(node.kind==="leaf")return {kind:"leaf",windows:node.windows.map(w=>String(w.internalId)),active:node.active};
        return {kind:"split",axis:node.axis,ratio:node.ratio,first:snapshotTree(node.first),second:snapshotTree(node.second)};
    }
    function saveSnapshot() {
        const snapshot={schema:1,gap,windows:snapshotWindows(),monitors:monitors.map(m=>({name:m.name,area:m.area,root:snapshotTree(m.root)}))};
        snapshotSave.arguments=[JSON.stringify(snapshot)];snapshotSave.call();
    }
    function restorePendingWindow(w) {
        if(!tileable(w)||slotOf(w))return false;
        const pending=pendingSnapshot;
        const item=pending.entries.find(e=>!e.window&&e.app.app===identity(w)&&e.app.title===String(w.caption))||
            pending.entries.find(e=>!e.window&&e.app.app===identity(w));
        if(!item)return false;
        item.window=w;identities.set(w,identity(w));floating.delete(w);
        if(item.slot)assign(item.slot,w);
        else {floating.add(w);if(visibleHere(w))placeWindow(w,item.app.rect);}
        snapshotRestored.arguments=[JSON.stringify({gap,missing:pending.entries.filter(e=>!e.window).length})];snapshotRestored.call();
        return true;
    }
    function restoreSnapshot(json) {
        if(interactiveWindows.size){console.log("Tilekeep: finish dragging before loading a snapshot");return;}
        const saved=JSON.parse(json),available=Workspace.stackingOrder.filter(w=>tileable(w)),used=new Set(),byToken=new Map();
        const entries=saved.windows.map(app=>{
            const w=available.find(w=>!used.has(w)&&identity(w)===app.app&&String(w.internalId)===app.token)||available.find(w=>!used.has(w)&&identity(w)===app.app&&String(w.caption)===app.title)||available.find(w=>!used.has(w)&&identity(w)===app.app);
            if(w)used.add(w);const e={app,window:w||null,slot:null};byToken.set(app.token,e);return e;
        });
        function tree(n,parent) {
            if(n.kind==="leaf") {
                const s=leaf();s.parent=parent;
                for(const token of n.windows){const e=byToken.get(token);if(!e)continue;e.slot=s;if(e.window)assign(s,e.window);}
                s.active=Math.min(n.active,Math.max(0,s.windows.length-1));return s;
            }
            const s={kind:"split",axis:n.axis,ratio:n.ratio,parent};s.first=tree(n.first,s);s.second=tree(n.second,s);return s;
        }
        syncMonitors();floating.clear();gap=saved.gap;
        for(let i=0;i<monitors.length;i++) {
            const m=monitors[i],old=saved.monitors.find(s=>s.name===m.name)||saved.monitors[i];
            m.root=old?tree(old.root,null):leaf();
        }
        // Extra windows are left open at their existing positions, outside the
        // restored layout. A missing saved app can claim its slot when it opens.
        for(const w of available)if(!slotOf(w))floating.add(w);
        for(const e of entries)if(e.window&&e.app.floating){floating.add(e.window);if(visibleHere(e.window))placeWindow(e.window,e.app.rect);}
        pendingSnapshot={entries};apply();
        snapshotRestored.arguments=[JSON.stringify({gap,missing:entries.filter(e=>!e.window).length})];snapshotRestored.call();
    }

    function start() {
        syncMonitors();
        for(const w of Workspace.stackingOrder) { connectWindow(w); appeared(w); }
        focused=Workspace.activeWindow; apply();
        console.log("Tilekeep: Plasma Wayland backend started; gap",gap,"dry-run",dryRun);
    }

    Component.onCompleted: start()
    Component.onDestruction: {
        placementDeadline.stop();placementSpacing.stop();recoveryTimer.stop();workAreaTimer.stop();hidePreview();
        for(const w of Array.from(windowConnections.keys())) disconnectWindow(w);
    }

    Connections {
        target: Workspace
        function onWindowAdded(w) { root.connectWindow(w);if(root.appeared(w))root.apply(); }
        function onWindowRemoved(w) { root.disconnectWindow(w);root.vanished(w);root.apply(); }
        function onWindowActivated(w) { if(w){if(root.appeared(w))root.apply();root.focused=w;const f=root.slotOf(w);if(f)f[1].active=f[1].windows.indexOf(w);} }
        function onScreensChanged() { root.syncMonitors();root.apply(); }
        function onCurrentDesktopChanged() { root.apply(); }
        function onCurrentActivityChanged() { root.apply(); }
    }

    Timer { id: placementDeadline; interval: 1000; repeat: false; onTriggered: root.placementTimedOut() }
    Timer { id: placementSpacing; interval: 100; repeat: false; onTriggered: root.placeNextWindow() }
    Timer { id: recoveryTimer; interval: 5000; running: !root.dryRun; repeat: true; onTriggered: root.retryDeferredPlacements() }
    // KWin's scripting API has no work-area-changed signal. Panel visibility,
    // position and size can change without screensChanged being emitted.
    Timer { id: workAreaTimer; interval: 500; running: true; repeat: true; onTriggered: root.workAreasChanged() }

    DBusCall { id: unloadCall }
    DBusCall { id: snapshotSave; service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"SaveSnapshot";onFailed:console.log("Tilekeep: snapshot save failed") }
    DBusCall { id: snapshotLoad; service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"ReadSnapshot";onFinished:(returnValue)=>root.restoreSnapshot(returnValue[0]);onFailed:console.log("Tilekeep: snapshot load failed") }
    DBusCall { id: snapshotRestored;service:"com.userfirst.Tilekeep";path:"/Tilekeep";dbusInterface:"com.userfirst.Tilekeep";method:"SnapshotRestored";onFailed:console.log("Tilekeep: snapshot status could not be saved") }

    // KWin hides its shared Workspace outline after emitting each drag step.
    // Keep our own surface alive, changing geometry only when the target changes.
    // The outline hint keeps it in KWin's overlay layer, out of application lists.
    Window {
        id: dragPreview
        readonly property bool __kwin_outline: true
        // The script's root Item is not in a visible application window.
        // Do not defer showing this surface until that implicit parent appears.
        transientParent: null
        flags: Qt.BypassWindowManagerHint | Qt.FramelessWindowHint |
            Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
        color: "transparent"
        visible: false
        KSvg.FrameSvgItem { anchors.fill: parent; imagePath: "widgets/translucentbackground" }
    }

    ShortcutHandler { name: "TilekeepCompact"; text: "Tilekeep: compact monitor"; sequence: "Meta+Shift+K"; onActivated: { if(root.paused)return;const m=root.monitorForOutput(Workspace.screenAt(Workspace.cursorPos)); if(m){root.compact(m);root.apply();} } }
    ShortcutHandler { name: "TilekeepRetile"; text: "Tilekeep: re-tile windows"; sequence: "Meta+Shift+L"; onActivated: { root.syncMonitors();root.apply(); } }
    ShortcutHandler { name: "TilekeepFloat"; text: "Tilekeep: toggle floating"; sequence: "Meta+Shift+F"; onActivated: { if(root.paused)return;const w=Workspace.activeWindow;if(!w)return;if(root.floating.has(w)){root.floating.delete(w);root.appeared(w);}else{root.detach(w);root.floating.add(w);}root.apply(); } }
    // A fresh action id avoids loading the old saved Meta+Shift+S binding,
    // which conflicts with Spectacle's default screenshot shortcut.
    ShortcutHandler { name: "TilekeepStackAtCursor"; text: "Tilekeep: stack at cursor"; sequence: "Meta+Shift+G"; onActivated: root.stackAtCursor() }
    ShortcutHandler { name: "TilekeepNext"; text: "Tilekeep: next stacked window"; sequence: "Meta+Shift+N"; onActivated: root.cycle(1) }
    ShortcutHandler { name: "TilekeepPrevious"; text: "Tilekeep: previous stacked window"; sequence: "Meta+Shift+B"; onActivated: root.cycle(-1) }
    ShortcutHandler { name: "TilekeepQuit"; text: "Tilekeep: quit"; sequence: "Meta+Shift+Q"; onActivated: root.stop() }
    ShortcutHandler { name: "TilekeepPause"; text: "Tilekeep: pause tiling"; onActivated: root.setPaused(true) }
    ShortcutHandler { name: "TilekeepResume"; text: "Tilekeep: resume tiling"; onActivated: root.setPaused(false) }
    ShortcutHandler {name:"TilekeepSaveSnapshot";text:"Tilekeep: save snapshot";onActivated:root.saveSnapshot()}
    ShortcutHandler {name:"TilekeepLoadSnapshot";text:"Tilekeep: load snapshot";onActivated:{snapshotLoad.arguments=[JSON.stringify(root.snapshotWindows())];snapshotLoad.call();}}
    Instantiator {
        model: 65
        delegate: ShortcutHandler {
            required property int index
            name: "TilekeepSetGap" + index
            text: "Tilekeep: set gap to " + index
            onActivated: { root.gap=index;root.apply(); }
        }
    }
}
