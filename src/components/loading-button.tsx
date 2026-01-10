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
      className={cn("grid place-items-center", className)}
      {...props}
      disabled={props.disabled || isLoading}
    >
      {isLoading ? (
        <LuLoaderCircle className="h-4 w-4 animate-spin" />
      ) : (
        props.children
      )}
    </UIButton>
  );
};
