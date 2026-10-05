// Product Manager routines (DIR-78): presentation helpers. The service owns
// scheduling; these format triggers and due times in the routine's timezone.
import type { Routine, RoutineNotice, RoutineOccurrence, RoutineTrigger } from "./api";

export const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

export function triggerLabel(t: RoutineTrigger): string {
  const when = t.kind === "weekly" ? `Weekly on ${t.weekday ? t.weekday[0].toUpperCase() + t.weekday.slice(1) : "?"}` : "Daily";
  return `${when} at ${t.time} (${t.timezone})`;
}

/** A due time shown in the routine's own timezone, e.g. "Mon, 5 Oct 2026, 09:00". */
export function dueLabel(at: number | null, timezone: string): string {
  if (!at) return "Not scheduled";
  try {
    return new Intl.DateTimeFormat("en-GB", {
      timeZone: timezone,
      weekday: "short",
      day: "numeric",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).format(new Date(at * 1000));
  } catch {
    return new Date(at * 1000).toISOString();
  }
}

/** Why a routine is not running on schedule, if anything. */
export function routineState(r: Routine, restoredAt: number | null): string {
  if (r.status === "retired") return "Retired";
  if (r.status === "paused") return "Paused";
  if (restoredAt && (!r.activated_at || r.activated_at < restoredAt)) return "Paused after restore — reactivate";
  if (r.held_reason) return `Held: ${r.held_reason}`;
  return "Active";
}

export function latestOccurrences(all: RoutineOccurrence[], routineId: string, limit = 10): RoutineOccurrence[] {
  return all.filter((o) => o.routine_id === routineId).sort((a, b) => b.created_at - a.created_at).slice(0, limit);
}

export function openNotices(all: RoutineNotice[], routineId?: string): RoutineNotice[] {
  return all.filter((n) => !n.acknowledged && (!routineId || n.routine_id === routineId)).sort((a, b) => b.at - a.at);
}
