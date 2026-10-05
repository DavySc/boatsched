export interface Me {
  member_id: number;
  name: string | null;
  is_admin: boolean;
}

export interface AppConfig {
  boats: number;
  levels: string[];
  restricted: string[];
  diverse: string[];
  adj?: number;
  db: boolean;
}

export interface Season {
  id: number;
  year: number;
}

export interface ScheduleMeta {
  id: number;
  created: string | null;
  start_date: string | null;
}

export interface Row {
  day: string;
  boat: number;
  name: string;
  level: string;
}

export interface Stats {
  mixed: number;
  distinct: number;
  repeats: number;
  status_A?: string;
  status_B?: string;
  status_C?: string;
}

export interface CellRef {
  name: string;
  day: string;
}

export interface Constraints {
  absences: CellRef[];
  locks: (CellRef & { boat: number })[];
}

export interface ScheduleData {
  rows: Row[];
  start_date: string | null;
  constraints: Constraints;
}

export interface JobStatus {
  status: "running" | "done" | "error";
  result?: Row[];
  stats?: Stats | null;
  schedule_id?: number;
  error?: string;
}

export interface Instructor {
  id: number;
  name: string;
  has_password: boolean;
  is_admin: boolean;
}

export interface Member {
  id: number;
  name: string;
}

export interface Boat {
  id: number;
  name: string | null;
}

export interface Progression {
  seasons: Season[];
  members: { id: number; name: string; levels: Record<string, string> }[];
}

export interface CommitResult {
  ok: boolean;
  conflicts?: { name: string; day: string; boats: number[] }[];
}

export interface SkillItem {
  id: number;
  label: string;
  subskills?: { id: number; label: string }[];
}
export interface SkillCategory {
  category: string;
  eval_type: "standard" | "overall" | string;
  skills: SkillItem[];
}
export type SkillCatalog = Record<string, SkillCategory[]>;

export interface DayProgress {
  skippers: number[];
  records: Record<string, string>;
}
export interface StudentProgress {
  member_id: number;
  name?: string;
  days: Record<string, DayProgress>;
  notes: string;
}
