export const metadata = {
  title: "Tirea AI SDK Demo",
  description: "E2E test frontend for tirea-agentos-server",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
