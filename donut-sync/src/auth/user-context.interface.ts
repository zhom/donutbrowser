export interface UserContext {
  mode: "self-hosted" | "cloud";
  prefix: string; // '' for self-hosted, 'users/{id}/' for cloud
  teamPrefix: string | null; // 'teams/{id}/' or null
  profileLimit: number; // 0 for unlimited (self-hosted)
}
