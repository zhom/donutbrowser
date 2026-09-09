import type { CookieBotSchedule, CookieBotSlot } from "./cookie-bot";

export function scheduleSlots(schedule: {
  slots?: CookieBotSlot[];
  run_at_minute: number;
  days_mask: number;
}): CookieBotSlot[] {
  return schedule.slots?.length
    ? schedule.slots
    : [
        {
          days_mask: schedule.days_mask,
          run_at_minute: schedule.run_at_minute,
        },
      ];
}

export interface ScheduleLane {
  id: string;
  schedule: CookieBotSchedule;
  minute: number;
  days: number;
  duration: number;
  overlaps: number;
}

function intersects(a: ScheduleLane, b: ScheduleLane): boolean {
  if (
    !a.schedule.enabled ||
    !b.schedule.enabled ||
    a.schedule.blocked_by ||
    b.schedule.blocked_by
  )
    return false;
  for (let day = 0; day < 7; day++) {
    if (!(a.days & (1 << day))) continue;
    const start = day * 1440 + a.minute;
    for (let otherDay = 0; otherDay < 7; otherDay++) {
      if (!(b.days & (1 << otherDay))) continue;
      for (const week of [-10080, 0, 10080]) {
        const otherStart = otherDay * 1440 + b.minute + week;
        if (start < otherStart + b.duration && otherStart < start + a.duration)
          return true;
      }
    }
  }
  return false;
}

/** Wall-clock reservations are compared only within the same timezone. */
export function scheduleLanes(
  schedules: CookieBotSchedule[],
): Map<string, ScheduleLane[]> {
  const groups = new Map<string, ScheduleLane[]>();
  for (const schedule of schedules) {
    const lanes = groups.get(schedule.timezone) ?? [];
    // Coincident slots dispatch once, even when their weekday masks overlap.
    const minutes = new Map<number, number>();
    for (const slot of scheduleSlots(schedule)) {
      minutes.set(
        slot.run_at_minute,
        (minutes.get(slot.run_at_minute) ?? 0) | slot.days_mask,
      );
    }
    let slotIndex = 0;
    for (const [minute, days] of minutes) {
      lanes.push({
        id: `${schedule.owner_user_id ?? ""}:${schedule.profile_id}:${slotIndex++}`,
        schedule,
        minute,
        days,
        duration: Math.max(0, schedule.max_minutes),
        overlaps: 0,
      });
    }
    groups.set(schedule.timezone, lanes);
  }
  for (const lanes of groups.values()) {
    lanes.sort(
      (a, b) =>
        a.minute - b.minute ||
        a.schedule.profile_name.localeCompare(b.schedule.profile_name),
    );
    for (let index = 0; index < lanes.length; index++) {
      for (let other = index + 1; other < lanes.length; other++) {
        if (intersects(lanes[index], lanes[other])) {
          lanes[index].overlaps++;
          lanes[other].overlaps++;
        }
      }
    }
  }
  return groups;
}
