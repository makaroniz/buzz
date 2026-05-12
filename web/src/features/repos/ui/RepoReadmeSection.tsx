import { FileText } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { useReadme } from "../use-git-browse";

const markdownComponents = {
  h1: ({ children }: { children?: React.ReactNode }) => (
    <h1 className="mb-4 mt-6 border-b border-border pb-2 text-2xl font-semibold first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }: { children?: React.ReactNode }) => (
    <h2 className="mb-3 mt-6 border-b border-border pb-2 text-xl font-semibold first:mt-0">
      {children}
    </h2>
  ),
  h3: ({ children }: { children?: React.ReactNode }) => (
    <h3 className="mb-2 mt-4 text-lg font-semibold first:mt-0">{children}</h3>
  ),
  h4: ({ children }: { children?: React.ReactNode }) => (
    <h4 className="mb-2 mt-4 text-base font-semibold first:mt-0">{children}</h4>
  ),
  p: ({ children }: { children?: React.ReactNode }) => (
    <p className="my-3 leading-relaxed first:mt-0 last:mb-0">{children}</p>
  ),
  a: ({ href, children }: { href?: string; children?: React.ReactNode }) => (
    <a
      href={href}
      className="text-primary underline underline-offset-4 hover:text-primary/80"
      target="_blank"
      rel="noopener noreferrer"
    >
      {children}
    </a>
  ),
  ul: ({ children }: { children?: React.ReactNode }) => (
    <ul className="my-3 list-disc space-y-1 pl-6 marker:text-muted-foreground">
      {children}
    </ul>
  ),
  ol: ({ children }: { children?: React.ReactNode }) => (
    <ol className="my-3 list-decimal space-y-1 pl-6 marker:text-muted-foreground">
      {children}
    </ol>
  ),
  li: ({ children }: { children?: React.ReactNode }) => (
    <li className="leading-relaxed [&_p]:my-1">{children}</li>
  ),
  blockquote: ({ children }: { children?: React.ReactNode }) => (
    <blockquote className="my-3 border-l-4 border-border pl-4 italic text-muted-foreground">
      {children}
    </blockquote>
  ),
  code: ({
    children,
    className,
  }: {
    children?: React.ReactNode;
    className?: string;
  }) => {
    const isBlock =
      typeof className === "string" && className.includes("language-");
    if (isBlock) {
      return <code className="block text-[13px] leading-6">{children}</code>;
    }
    return (
      <code className="rounded bg-muted px-1.5 py-0.5 text-[13px]">
        {children}
      </code>
    );
  },
  pre: ({ children }: { children?: React.ReactNode }) => (
    <pre className="my-3 overflow-x-auto rounded-md border border-border bg-muted/60 px-4 py-3 text-sm">
      {children}
    </pre>
  ),
  hr: () => <hr className="my-6 border-border" />,
  table: ({ children }: { children?: React.ReactNode }) => (
    <div className="my-3 overflow-x-auto rounded-md border border-border">
      <table className="w-full border-collapse text-left text-sm">
        {children}
      </table>
    </div>
  ),
  th: ({ children }: { children?: React.ReactNode }) => (
    <th className="border-b border-border bg-muted/60 px-3 py-2 font-semibold">
      {children}
    </th>
  ),
  td: ({ children }: { children?: React.ReactNode }) => (
    <td className="border-t border-border px-3 py-2">{children}</td>
  ),
  strong: ({ children }: { children?: React.ReactNode }) => (
    <strong className="font-semibold">{children}</strong>
  ),
  img: ({ src, alt }: { src?: string; alt?: string }) => (
    <img src={src} alt={alt} className="my-3 max-w-full rounded-md" />
  ),
};

export function RepoReadmeSection({
  repoId,
  owner,
  gitRef,
}: {
  repoId: string;
  owner: string;
  gitRef: string;
}) {
  const { data: readme, isLoading, error } = useReadme(repoId, owner, gitRef);

  if (isLoading) return null;
  if (error) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <FileText className="h-4 w-4" />
          README
        </h2>
        <p className="text-sm text-destructive">Failed to load README</p>
      </div>
    );
  }
  if (!readme) return null;

  return (
    <div className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <FileText className="h-4 w-4" />
        {readme.filename}
      </h2>
      <div className="rounded-md border border-border bg-muted/30 p-6 text-sm text-foreground/90">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={markdownComponents}
        >
          {readme.content}
        </ReactMarkdown>
      </div>
    </div>
  );
}
