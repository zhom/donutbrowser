export interface UserContext {
  mode: "self-hosted" | "cloud";
  prefix: string; // '' for self-hosted, 'users/{id}/' for cloud
  teamPrefix: string | null; // 'teams/{id}/' or null
  profileLimit: number; // 0 for unlimited (self-hosted)
  teamProfileLimit: number; // 0 for unlimited or non-team users
}
