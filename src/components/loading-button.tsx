import { LuLoaderCircle } from "react-icons/lu";
import { type ButtonProps, Button as UIButton } from "./ui/button";
type Props = ButtonProps & {
  isLoading: boolean;
  "aria-label"?: string;
};
export const LoadingButton = ({ isLoading, ...props }: Props) => {
  return (
    <UIButton className="grid place-items-center" {...props}>
      {isLoading ? (
        <LuLoaderCircle className="h-4 w-4 animate-spin" />
      ) : (
        props.children
      )}
    </UIButton>
  );
};
