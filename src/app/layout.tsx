// Bundled with the app: the build and the app never fetch fonts from a CDN.
import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import "@/styles/globals.css";
import "flag-icons/css/flag-icons.min.css";
import { ClientProviders } from "@/components/client-providers";

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body
        className={`${GeistSans.variable} ${GeistMono.variable} antialiased overflow-hidden bg-background`}
      >
        <ClientProviders>{children}</ClientProviders>
      </body>
    </html>
  );
}
