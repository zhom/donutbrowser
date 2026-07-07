export interface UserContext {
  mode: "self-hosted" | "cloud";
  // The EFFECTIVE namespace for this request: '' for self-hosted, and for cloud
  // either the user's own 'users/{sub}/' or, for a team member, the shared team
  // owner's 'users/{ownerId}/' — resolved server-side by the AuthGuard from the
  // backend (never carried in the JWT). All key scoping uses this directly.
  prefix: string;
  profileLimit: number; // 0 for unlimited (self-hosted); effective (team) limit for team members
  sub?: string; // the authenticated user id (cloud only)
}
