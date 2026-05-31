"use client";

import { AnimatePresence, type HTMLMotionProps, motion } from "motion/react";
import { Dialog as DialogPrimitive } from "radix-ui";
import type * as React from "react";
import { useTranslation } from "react-i18next";
import { RxCross2 } from "react-icons/rx";

import { useControlledState } from "@/hooks/use-controlled-state";
import { getStrictContext } from "@/lib/get-strict-context";
import { cn } from "@/lib/utils";
import { WindowDragArea } from "../window-drag-area";

type DialogContextType = {
  isOpen: boolean;
  setIsOpen: DialogProps["onOpenChange"];
  subPage: boolean;
  container: HTMLElement | null | undefined;
};

const [DialogProvider, useDialog] =
  getStrictContext<DialogContextType>("DialogContext");

type DialogProps = React.ComponentProps<typeof DialogPrimitive.Root> & {
  /** Render in a portal container as an in-flow sub-page instead of a centered modal. */
  subPage?: boolean;
  /** Portal container target. Required when subPage=true; ignored otherwise. */
  container?: HTMLElement | null;
};

function Dialog({ subPage, container, children, ...props }: DialogProps) {
  const [isOpen, setIsOpen] = useControlledState({
    value: props?.open,
    defaultValue: props?.defaultOpen,
    onChange: props?.onOpenChange,
  });

  return (
    <DialogProvider
      value={{
        isOpen,
        setIsOpen,
        subPage: !!subPage,
        container: container ?? undefined,
      }}
    >
      {/* In sub-page mode the Dialog isn't a modal — it's an in-flow page.
          Forcing `modal={false}` prevents Radix from locking pointer-events
          and aria-hiding everything outside the dialog. Children are passed
          explicitly (not via spread) so React doesn't have to guess where
          the JSX subtree should mount. */}
      <DialogPrimitive.Root
        data-slot="dialog"
        {...props}
        modal={subPage ? false : props.modal}
        onOpenChange={setIsOpen}
      >
        {children}
      </DialogPrimitive.Root>
    </DialogProvider>
  );
}

type DialogTriggerProps = React.ComponentProps<typeof DialogPrimitive.Trigger>;

function DialogTrigger(props: DialogTriggerProps) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

type DialogPortalProps = Omit<
  React.ComponentProps<typeof DialogPrimitive.Portal>,
  "forceMount"
>;

function DialogPortal(props: DialogPortalProps) {
  const { isOpen, container } = useDialog();

  return (
    <AnimatePresence>
      {isOpen && (
        <DialogPrimitive.Portal
          data-slot="dialog-portal"
          forceMount
          container={container ?? props.container}
          {...props}
        />
      )}
    </AnimatePresence>
  );
}

type DialogOverlayProps = Omit<
  React.ComponentProps<typeof DialogPrimitive.Overlay>,
  "forceMount" | "asChild"
> &
  HTMLMotionProps<"div">;

function DialogOverlay({
  className,
  transition = { duration: 0.2, ease: "easeInOut" },
  ...props
}: DialogOverlayProps) {
  return (
    <DialogPrimitive.Overlay data-slot="dialog-overlay" asChild forceMount>
      <motion.div
        key="dialog-overlay"
        initial={{ opacity: 0, filter: "blur(4px)" }}
        animate={{ opacity: 1, filter: "blur(0px)" }}
        exit={{ opacity: 0, filter: "blur(4px)" }}
        transition={transition}
        className={cn("fixed inset-0 z-9999 bg-background/50", className)}
        {...props}
      >
        {/* Keep the OS title-bar zone draggable while a modal is open — the
            overlay otherwise covers the native drag region. `data-window-drag-area`
            stops Radix from treating a drag here as an outside-click dismiss. */}
        <div
          data-tauri-drag-region
          data-window-drag-area="true"
          aria-hidden="true"
          className="absolute inset-x-0 top-0 h-11"
        />
        <WindowDragArea />
      </motion.div>
    </DialogPrimitive.Overlay>
  );
}

type DialogContentProps = Omit<
  React.ComponentProps<typeof DialogPrimitive.Content>,
  "forceMount" | "asChild"
> &
  HTMLMotionProps<"div"> & {
    /**
     * Suppress the built-in top-right close X. Use when the dialog renders
     * its own header bar with a custom close control to avoid two X buttons
     * stacking near the corner.
     */
    hideClose?: boolean;
    /**
     * When false, the user cannot dismiss the dialog — Escape and outside
     * clicks are ignored and the close X is hidden. Use for steps the user
     * must complete to progress (e.g. required onboarding, a blocking
     * download). The dialog can still be closed programmatically via `open`.
     */
    dismissible?: boolean;
  };

function SubPageContent({
  children,
}: {
  className?: string;
  children?: React.ReactNode;
}) {
  const { isOpen } = useDialog();
  if (!isOpen) return null;
  // Inline styles deliberately override any className the caller passed
  // for the modal mode (max-w-*, max-h-*, my-*). tailwind-merge inside the
  // shared dialog wrappers turned out to be unreliable when both classnames
  // and !important variants competed — inline styles guarantee the layout.
  return (
    <motion.div
      data-slot="sub-page"
      data-sub-page="true"
      initial={false}
      animate={{ opacity: 1 }}
      style={{
        position: "relative",
        display: "flex",
        flexDirection: "column",
        flex: "1 1 0%",
        minHeight: 0,
        width: "100%",
        maxWidth: "none",
        height: "100%",
        maxHeight: "none",
        margin: 0,
        padding: 12,
        gap: 12,
        overflow: "auto",
        background: "var(--background)",
      }}
    >
      {children}
    </motion.div>
  );
}

function DialogContent({
  className,
  children,
  onOpenAutoFocus,
  onCloseAutoFocus,
  onEscapeKeyDown,
  onPointerDownOutside,
  onInteractOutside,
  transition,
  hideClose,
  dismissible = true,
  ...props
}: DialogContentProps) {
  const { t } = useTranslation();
  const { subPage } = useDialog();

  if (subPage) {
    return <SubPageContent>{children}</SubPageContent>;
  }

  return (
    <DialogPortal data-slot="dialog-portal">
      <DialogOverlay />
      <DialogPrimitive.Content
        asChild
        forceMount
        onOpenAutoFocus={onOpenAutoFocus}
        onCloseAutoFocus={onCloseAutoFocus}
        onEscapeKeyDown={(event) => {
          if (!dismissible) event.preventDefault();
          onEscapeKeyDown?.(event);
        }}
        onPointerDownOutside={onPointerDownOutside}
        onInteractOutside={(event) => {
          if (!dismissible) {
            event.preventDefault();
            return;
          }
          const target = event.target as HTMLElement | null;
          if (target?.closest('[data-window-drag-area="true"]')) {
            event.preventDefault();
          }
          onInteractOutside?.(event);
        }}
      >
        <motion.div
          key="dialog-content"
          data-slot="dialog-content"
          // Open/close motion modeled on transitions.dev's modal: a subtle
          // scale from 0.96 → 1 with opacity, eased with cubic-bezier(0.22, 1,
          // 0.36, 1). Open is 250ms; close is a quicker 150ms. The centering
          // translate stays in `style` so `scale` animates around the center
          // without fighting the transform-based positioning.
          style={{ transformOrigin: "center" }}
          initial={{ opacity: 0, scale: 0.96 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{
            opacity: 0,
            scale: 0.96,
            transition: transition ?? {
              duration: 0.15,
              ease: [0.22, 1, 0.36, 1],
            },
          }}
          transition={
            transition ?? { duration: 0.25, ease: [0.22, 1, 0.36, 1] }
          }
          className={cn(
            "bg-background fixed top-[50%] left-[50%] z-10000 grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 rounded-lg border p-6 shadow-lg",
            className,
          )}
          {...props}
        >
          {children}
          {!hideClose && dismissible && (
            <DialogPrimitive.Close className="cursor-pointer ring-offset-background focus:ring-ring data-[state=open]:bg-accent data-[state=open]:text-muted-foreground absolute top-4 right-4 rounded-xs opacity-70 transition-opacity hover:opacity-100 focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4">
              <RxCross2 />
              <span className="sr-only">{t("common.buttons.close")}</span>
            </DialogPrimitive.Close>
          )}
        </motion.div>
      </DialogPrimitive.Content>
    </DialogPortal>
  );
}

type DialogCloseProps = React.ComponentProps<typeof DialogPrimitive.Close>;

function DialogClose(props: DialogCloseProps) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn("flex flex-col gap-2 text-center sm:text-left", className)}
      {...props}
    />
  );
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn(
        "flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    />
  );
}

function DialogTitle({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("text-lg font-semibold leading-none", className)}
      {...props}
    />
  );
}

function DialogDescription({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
