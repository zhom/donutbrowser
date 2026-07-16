export const MOTION_EASE_OUT: [number, number, number, number] = [
  0.23, 1, 0.32, 1,
];

export const MOTION_SPRING_POSITION = {
  type: "spring" as const,
  stiffness: 500,
  damping: 30,
  mass: 0.5,
};
