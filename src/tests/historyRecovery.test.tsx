// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {TransferHistory} from "../components/history/TransferHistory";
const mocks=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("@tauri-apps/api/core",()=>({invoke:mocks.invoke}));
vi.mock("../utils/tauri",()=>({isTauri:()=>true}));
it("inspects the selected recovery job and blocks resume while outcomes are uncertain",async()=>{
    (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
    const entry={job_id:"recover-one",snapshot_id:"snap_recover-one",source_service:"spotify",target_service:"ytmusic",source_playlist_name:"Fixture",target_playlist_id:"fixture",is_new_playlist:false,total_tracks:3,matched_tracks:3,added_tracks_count:1,pre_existing_count:0,is_rolled_back:false,created_at:0,job_state:"recovery_required",recovery_required:true,last_error:null};
    mocks.invoke.mockImplementation(async(command:string)=>{
        if(command==="get_transfer_history")return[entry];
        if(command==="get_storage_recovery_notice")return null;
        if(command==="inspect_transfer_job")return{job_id:entry.job_id,state:"manual_review",event_seq:4,counts:{selected:3,confirmed:1,failed:0,skipped:0,uncertain:1,unsent:1},verification:"unavailable",operations:[],can_resume:false,next_action:"Review uncertain operations"};
        throw new Error(`Unexpected command ${command}`);
    });
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    try{
        await act(async()=>{root.render(<TransferHistory/>);});
        const inspect=Array.from(container.querySelectorAll("button")).find(b=>b.textContent?.includes("Inspect recovery"));
        expect(inspect).toBeTruthy();
        await act(async()=>inspect!.click());
        expect(mocks.invoke).toHaveBeenCalledWith("inspect_transfer_job",{jobId:"recover-one"});
        expect(container.textContent).toContain("1 uncertain");
        const resume=Array.from(container.querySelectorAll("button")).find(b=>b.textContent?.includes("Resume recorded"));
        expect(resume?.disabled).toBe(true);
    }finally{await act(async()=>root.unmount());container.remove();}
});

it("offers explicit attributed rollback after inspection and refreshes its acknowledgment",async()=>{
    let rolledBack=false;
    const entry={job_id:"owned-job",snapshot_id:"snap_owned-job",source_service:"spotify",target_service:"ytmusic",source_playlist_name:"Fixture",target_playlist_id:"fixture",is_new_playlist:false,total_tracks:1,matched_tracks:1,added_tracks_count:1,pre_existing_count:1,is_rolled_back:false,created_at:0,job_state:"manual_review",recovery_required:true,last_error:null};
    mocks.invoke.mockImplementation(async(command:string)=>{
        if(command==="get_transfer_history")return[{...entry,is_rolled_back:rolledBack,job_state:rolledBack?"rolled_back":"manual_review",recovery_required:!rolledBack}];if(command==="get_storage_recovery_notice")return null;
        if(command==="execute_snapshot_rollback"){rolledBack=true;return;}
        if(command==="inspect_transfer_job")return{job_id:entry.job_id,state:rolledBack?"rolled_back":"manual_review",event_seq:rolledBack?5:4,counts:{selected:1,confirmed:1,failed:0,skipped:0,uncertain:0,unsent:0},verification:rolledBack?"verified":"unavailable",operations:[],can_resume:false,next_action:"Inspect recorded outcomes"};
        throw new Error(`Unexpected command ${command}`);
    });
    const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
    try {
        await act(async()=>{root.render(<TransferHistory/>);});
        const find=(text:string)=>Array.from(document.body.querySelectorAll("button")).find(b=>b.textContent?.includes(text));
        await act(async()=>find("Inspect recovery")!.click());
        const rollback=find("Rollback recorded additions");expect(rollback).toBeTruthy();
        await act(async()=>rollback!.click());
        expect(document.body.querySelector('[role="dialog"]')).toBeTruthy();
        await act(async()=>find("Confirm Rollback")!.click());
        expect(mocks.invoke).toHaveBeenCalledWith("execute_snapshot_rollback",{snapshotId:"snap_owned-job",deleteEntirePlaylist:false});
        expect(container.textContent).toContain("Rolled Back");
    }finally{await act(async()=>root.unmount());container.remove();}
});
