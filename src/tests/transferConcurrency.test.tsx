// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {TransferConfirmModal} from "../components/diff/TransferConfirmModal";
import {SettingsPage} from "../components/settings/SettingsPage";
vi.mock("../utils/tauri",()=>({isTauri:()=>false}));
it("uses one ordered playlist writer even when older preferences request eight",async()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  localStorage.setItem("soundshift.settings",JSON.stringify({concurrency:8}));
  const onConfirm=vi.fn();const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
  const track={id:"a",title:"fixture",artists:[],duration_ms:0,is_explicit:false,is_playable:true};
  try {
    await act(async()=>root.render(<TransferConfirmModal isOpen onClose={()=>{}} onConfirm={onConfirm} targetService="spotify" results={[{source_track:track,matched_track:track,status:"Exact",confidence:1,candidates:[]}]} selectedTrackIds={new Set(["a"])} />));
    await act(async()=>Array.from(document.body.querySelectorAll("button")).find(b=>b.textContent?.includes("Confirm & Start"))!.click());
    expect(onConfirm.mock.calls[0][0].concurrency).toBe(1);
    await act(async()=>root.render(<SettingsPage/>));
    const control=container.querySelector<HTMLInputElement>('input[aria-label="Concurrent workers"]')!;
    expect(control.disabled).toBe(true);expect(control.value).toBe("1");
  }finally{await act(async()=>root.unmount());container.remove();localStorage.clear();}
});
