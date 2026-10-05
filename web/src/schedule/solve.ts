import { api } from "../api";
import type { JobStatus } from "../types";

/** Poll a background solve job until it finishes; rejects with the solver's message. */
export async function waitForJob(id: string): Promise<JobStatus> {
  for (;;) {
    const j = await api<JobStatus>(`/status?id=${encodeURIComponent(id)}`);
    if (j.status === "done") return j;
    if (j.status === "error") throw new Error(j.error ?? "Indelen mislukt");
    await new Promise((r) => setTimeout(r, 1200));
  }
}
