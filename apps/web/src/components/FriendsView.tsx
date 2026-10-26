import { useState } from 'react'

type Tab = 'online' | 'all' | 'pending'

export function FriendsView() {
  const [activeTab, setActiveTab] = useState<Tab>('online')

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'

  const friends = [
    { id: 'alice', status: 'Online', color: 'bg-green-500' },
    { id: 'bob', status: 'Idle', color: 'bg-yellow-500' },
    { id: 'charlie', status: 'Offline', color: 'bg-gray-500' },
  ]

  return (
    <div className="flex flex-col h-full w-full max-w-[1000px] mx-auto">
      <h1 className="text-3xl font-extrabold tracking-tight mb-6 mt-1 text-[var(--text-main)]">
        Friends
      </h1>

      <div className="flex items-center gap-6 border-b border-[var(--bg-hover)] mb-4 pb-2">
        {(['online', 'all', 'pending'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={`text-[14px] font-medium capitalize transition-colors ${
              activeTab === tab 
                ? 'text-[var(--text-main)]' 
                : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'
            }`}
          >
            {tab}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto">
        {activeTab === 'pending' ? (
          <div className="text-[13px] text-[var(--text-muted)] py-4">No pending requests.</div>
        ) : (
          <div className="flex flex-col gap-2">
            {friends.filter(f => activeTab === 'online' ? f.status === 'Online' : true).map(f => (
              <div key={f.id} className="flex items-center justify-between p-3 rounded-lg hover:bg-[var(--bg-hover)] transition-colors group cursor-pointer">
                <div className="flex items-center gap-3">
                  <div className="relative">
                    <div className="w-9 h-9 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[13px] font-bold text-[var(--text-main)]">
                      {getInitial(f.id)}
                    </div>
                    <div className={`absolute bottom-0 right-0 w-3 h-3 ${f.color} border-2 border-[var(--bg-app)] rounded-full`}></div>
                  </div>
                  <div className="flex flex-col">
                    <span className="text-[14px] font-semibold text-[var(--text-main)]">{f.id}</span>
                    <span className="text-[11px] text-[var(--text-muted)]">{f.status}</span>
                  </div>
                </div>
                <div className="flex items-center gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button className="px-3 py-1.5 bg-[var(--bg-active)] hover:bg-[var(--text-muted)] text-[var(--text-main)] text-[12px] rounded-md font-medium transition-colors">
                    Message
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}