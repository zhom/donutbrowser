"use client";

import { LoadingButton } from "@/components/loading-button";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useVersionUpdater } from "@/hooks/use-version-updater";
import {
  LuCheckCheck,
  LuCircleAlert,
  LuClock,
  LuRefreshCw,
} from "react-icons/lu";

export function VersionUpdateSettings() {
  const {
    isUpdating,
    lastUpdateTime,
    timeUntilNextUpdate,
    updateProgress,
    triggerManualUpdate,
    formatTimeUntilUpdate,
    formatLastUpdateTime,
  } = useVersionUpdater();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex gap-2 items-center">
          <LuRefreshCw className="w-5 h-5" />
          Background Version Updates
        </CardTitle>
        <CardDescription>
          Browser versions are automatically checked every 3 hours in the
          background. New versions are cached and ready when you need them.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Current Status */}
        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <div className="flex gap-2 items-center text-sm font-medium">
              <LuClock className="w-4 h-4" />
              Last Update
            </div>
            <div className="text-sm text-muted-foreground">
              {formatLastUpdateTime(lastUpdateTime)}
            </div>
          </div>

          <div className="space-y-2">
            <div className="flex gap-2 items-center text-sm font-medium">
              <LuCheckCheck className="w-4 h-4" />
              Next Update
            </div>
            <div className="text-sm text-muted-foreground">
              {timeUntilNextUpdate <= 0
                ? "Now"
                : `In ${formatTimeUntilUpdate(timeUntilNextUpdate)}`}
            </div>
          </div>
        </div>

        {/* Progress indicator */}
        {isUpdating && updateProgress && (
          <Alert>
            <LuRefreshCw className="w-4 h-4 animate-spin" />
            <AlertTitle>Updating Browser Versions</AlertTitle>
            <AlertDescription>
              {updateProgress.current_browser ? (
                <>
                  Looking for updates for {updateProgress.current_browser} (
                  {updateProgress.completed_browsers}/
                  {updateProgress.total_browsers})
                </>
              ) : (
                "Starting version update..."
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* Manual update button */}
        <div className="flex justify-between items-center pt-2 border-t">
          <div className="space-y-1">
            <div className="text-sm font-medium">Manual Update</div>
            <div className="text-xs text-muted-foreground">
              Check for new browser versions now
            </div>
          </div>
          <LoadingButton
            isLoading={isUpdating}
            onClick={() => {
              void triggerManualUpdate();
            }}
            variant="outline"
            size="sm"
            disabled={isUpdating}
          >
            <LuRefreshCw className="mr-2 w-4 h-4" />
            {isUpdating ? "Updating..." : "Check Now"}
          </LoadingButton>
        </div>

        {/* Information about background updates */}
        <Alert>
          <LuCircleAlert className="w-4 h-4" />
          <AlertTitle>How it works</AlertTitle>
          <AlertDescription className="text-xs">
            • Version information is checked automatically every 3 hours
            <br />• New versions are added to the cache without removing old
            ones
            <br />• When creating profiles or changing versions, you&apos;ll see
            how many new versions were found
            <br />• This keeps the app responsive while ensuring you have the
            latest information
          </AlertDescription>
        </Alert>
      </CardContent>
    </Card>
  );
}
