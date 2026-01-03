export class StatRequestDto {
  key: string;
}

export class StatResponseDto {
  exists: boolean;
  lastModified?: string;
  size?: number;
}

export class PresignUploadRequestDto {
  key: string;
  contentType?: string;
  expiresIn?: number;
}

export class PresignUploadResponseDto {
  url: string;
  expiresAt: string;
}

export class PresignDownloadRequestDto {
  key: string;
  expiresIn?: number;
}

export class PresignDownloadResponseDto {
  url: string;
  expiresAt: string;
}

export class DeleteRequestDto {
  key: string;
  tombstoneKey?: string;
  deletedAt?: string;
}

export class DeleteResponseDto {
  deleted: boolean;
  tombstoneCreated: boolean;
}

export class ListRequestDto {
  prefix: string;
  maxKeys?: number;
  continuationToken?: string;
}

export class ListObjectDto {
  key: string;
  lastModified: string;
  size: number;
}

export class ListResponseDto {
  objects: ListObjectDto[];
  isTruncated: boolean;
  nextContinuationToken?: string;
}

export class SubscribeEventDto {
  type: "change" | "delete" | "ping";
  key?: string;
  lastModified?: string;
  size?: number;
}

// Batch presign DTOs
export class PresignUploadBatchItemDto {
  key: string;
  contentType?: string;
}

export class PresignUploadBatchRequestDto {
  items: PresignUploadBatchItemDto[];
  expiresIn?: number;
}

export class PresignUploadBatchItemResponseDto {
  key: string;
  url: string;
  expiresAt: string;
}

export class PresignUploadBatchResponseDto {
  items: PresignUploadBatchItemResponseDto[];
}

export class PresignDownloadBatchRequestDto {
  keys: string[];
  expiresIn?: number;
}

export class PresignDownloadBatchItemResponseDto {
  key: string;
  url: string;
  expiresAt: string;
}

export class PresignDownloadBatchResponseDto {
  items: PresignDownloadBatchItemResponseDto[];
}

// Delete prefix DTOs
export class DeletePrefixRequestDto {
  prefix: string;
  tombstoneKey?: string;
  deletedAt?: string;
}

export class DeletePrefixResponseDto {
  deletedCount: number;
  tombstoneCreated: boolean;
}
