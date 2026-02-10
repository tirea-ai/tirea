export const metadata = {
  title: "Uncarve AI SDK Demo",
  description: "E2E test frontend for carve-agentos-server",
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
