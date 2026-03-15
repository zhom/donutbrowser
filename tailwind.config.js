export default {
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        black: "#000000",
      },
      backgroundColor: {
        dark: "#000000",
      },
      keyframes: {
        wiggle: {
          "0%, 100%": { transform: "rotate(0deg)" },
          "25%": { transform: "rotate(-12deg)" },
          "75%": { transform: "rotate(12deg)" },
        },
      },
      animation: {
        wiggle: "wiggle 0.3s ease-in-out",
      },
    },
  },
};
