"use client";

import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { useFieldVariant } from "@/components/ui/field-variant";
import { cn } from "@/lib/utils";

const textareaVariants = cva(
  "flex min-h-[80px] w-full px-3 py-2 text-sm transition-[color,background-color,border-color,box-shadow] placeholder:text-muted-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50",
  {
    variants: {
      variant: {
        default:
          "rounded-md border border-input bg-background ring-offset-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        soft: "rounded-lg bg-foreground/5 hover:bg-foreground/7 focus-visible:bg-foreground/7 focus-visible:ring-2 focus-visible:ring-foreground/15 aria-invalid:bg-destructive/10 aria-invalid:ring-2 aria-invalid:ring-destructive/40",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export type TextareaProps = React.TextareaHTMLAttributes<HTMLTextAreaElement> &
  VariantProps<typeof textareaVariants>;

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, variant, ...props }, ref) => {
    const inherited = useFieldVariant();
    return (
      <textarea
        className={cn(
          textareaVariants({ variant: variant ?? inherited }),
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Textarea.displayName = "Textarea";

export { Textarea };
