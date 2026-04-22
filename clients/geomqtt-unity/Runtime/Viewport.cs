using System.Collections.Generic;

namespace Geomqtt
{
    public static class Viewport
    {
        /// <summary>Diff previous vs next topic sets.</summary>
        public static (List<string> toSubscribe, List<string> toUnsubscribe) Diff(
            HashSet<string> previous, HashSet<string> next)
        {
            var sub = new List<string>();
            var uns = new List<string>();
            foreach (var t in next)
                if (!previous.Contains(t)) sub.Add(t);
            foreach (var t in previous)
                if (!next.Contains(t)) uns.Add(t);
            return (sub, uns);
        }
    }
}
