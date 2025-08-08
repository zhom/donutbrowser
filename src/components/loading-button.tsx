import { LuLoaderCircle } from "react-icons/lu";
import {
  type RippleButtonProps as ButtonProps,
  RippleButton as UIButton,
} from "./ui/ripple";

type Props = ButtonProps & {
  isLoading: boolean;
  "aria-label"?: string;
};
export const LoadingButton = ({ isLoading, ...props }: Props) => {
  return (
    <UIButton
      className="grid place-items-center"
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
