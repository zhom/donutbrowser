export interface UserContext {
  // `self-hosted`: the static SYNC_TOKEN, the whole bucket. `cloud`: Donut's
  // own sync server. `account`: a Donut account on a server the customer runs,
  // with the same namespace, limit and team rules as `cloud`.
  mode: "self-hosted" | "cloud" | "account";
  // The EFFECTIVE namespace for this request: '' for self-hosted, and otherwise
  // either the user's own 'users/{sub}/' or, for a team member, the shared team
  // owner's 'users/{ownerId}/' — resolved server-side by the AuthGuard from the
  // backend (never carried in the JWT). All key scoping uses this directly.
  prefix: string;
  profileLimit: number; // 0 for unlimited (self-hosted); effective (team) limit for team members
  sub?: string; // the authenticated user id (cloud and account only)
  // The caller's role on the team whose namespace `prefix` is, from the
  // backend. A `launcher` may run profiles and sync what a session changed,
  // never change a profile's settings, proxies or groups.
  teamRole?: "owner" | "admin" | "member" | "launcher";
  // The profile groups this member may reach, when the owner limited them.
  // Absent means every group. `__ungrouped__` stands for profiles in no group.
  groupIds?: string[];
}
