import { MapPinOff } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

/** Explains a hash address that the console does not recognize. */
export function UnknownRouteView({ address }: { address: string | null }) {
  const path = address ? `#/${address}` : "that address";

  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <Card className="w-full max-w-lg">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <MapPinOff className="size-4" /> Page not found
          </CardTitle>
          <CardDescription>
            {path} does not name a page in this console. Check the address or return to Overview.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button render={<a href="#/overview" />}>Go to Overview</Button>
        </CardContent>
      </Card>
    </div>
  );
}
