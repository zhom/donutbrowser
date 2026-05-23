import { LuLoaderCircle } from "react-icons/lu";
import { cn } from "@/lib/utils";
import {
  type RippleButtonProps as ButtonProps,
  RippleButton as UIButton,
} from "./ui/ripple";

type Props = ButtonProps & {
  isLoading: boolean;
  "aria-label"?: string;
};
export const LoadingButton = ({ isLoading, className, ...props }: Props) => {
  return (
    <UIButton
      className={cn("inline-flex items-center justify-center", className)}
      {...props}
      disabled={props.disabled || isLoading}
    >
      {isLoading ? (
        <LuLoaderCircle className="size-4 animate-spin" />
      ) : (
        props.children
      )}
    </UIButton>
  );
};
