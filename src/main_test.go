package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"
)

func newTestServer(t *testing.T) *server {
	t.Helper()
	db, err := openDB(filepath.Join(t.TempDir(), "favorites.db"))
	if err != nil {
		t.Fatalf("openDB: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	return newServer(db, "test-token")
}

// seed inserts a favorite with the given fields.
func seed(t *testing.T, s *server, id string, useCount int, lastUsed int64) {
	t.Helper()
	_, err := s.db.Exec(
		`INSERT INTO favorites (id, url, preview, provider, title, added_at, last_used, use_count)
		 VALUES (?, ?, '', 'giphy', ?, ?, ?, ?)`,
		id, "https://example.com/"+id+".gif", id, time.Now().Unix(), lastUsed, useCount)
	if err != nil {
		t.Fatalf("seed %s: %v", id, err)
	}
}

func get(t *testing.T, s *server, path string) (*httptest.ResponseRecorder, []favorite) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, path, nil)
	req.Header.Set("X-Auth-Token", "test-token")
	rec := httptest.NewRecorder()
	s.serveMux().ServeHTTP(rec, req)
	var items []favorite
	if rec.Code == http.StatusOK {
		if err := json.Unmarshal(rec.Body.Bytes(), &items); err != nil {
			t.Fatalf("decode response: %v", err)
		}
	}
	return rec, items
}

func ids(items []favorite) []string {
	out := make([]string, 0, len(items))
	for _, f := range items {
		out = append(out, f.ID)
	}
	return out
}

// Ordering is use_count DESC, last_used DESC, id ASC; limit+offset windows it.
func TestListFavoritesLimitOffset(t *testing.T) {
	s := newTestServer(t)
	// Expected order: f3 (uc=3), f2 (uc=2), f1 (uc=1), b (uc=0, newer), a (uc=0, older).
	seed(t, s, "a", 0, 100)
	seed(t, s, "f1", 1, 100)
	seed(t, s, "f2", 2, 100)
	seed(t, s, "f3", 3, 100)
	seed(t, s, "b", 0, 200)

	rec, items := get(t, s, "/api/v1/favorites")
	want := []string{"f3", "f2", "f1", "b", "a"}
	if len(ids(items)) != len(want) {
		t.Fatalf("full list = %v, want %v", ids(items), want)
	}
	if got := rec.Header().Get("X-Total-Count"); got != "5" {
		t.Fatalf("X-Total-Count = %q, want 5", got)
	}

	_, items = get(t, s, "/api/v1/favorites?limit=2")
	got := ids(items)
	if len(got) != 2 || got[0] != "f3" || got[1] != "f2" {
		t.Fatalf("limit=2 = %v, want [f3 f2]", got)
	}

	_, items = get(t, s, "/api/v1/favorites?limit=2&offset=2")
	got = ids(items)
	if len(got) != 2 || got[0] != "f1" || got[1] != "b" {
		t.Fatalf("limit=2&offset=2 = %v, want [f1 b]", got)
	}

	// Offset past the end returns an empty (non-null) array.
	_, items = get(t, s, "/api/v1/favorites?limit=10&offset=50")
	if items == nil || len(items) != 0 {
		t.Fatalf("offset past end = %v, want empty non-nil array", items)
	}

	// Offset without limit still skips.
	_, items = get(t, s, "/api/v1/favorites?offset=4")
	got = ids(items)
	if len(got) != 1 || got[0] != "a" {
		t.Fatalf("offset=4 = %v, want [a]", got)
	}
}

func TestListFavoritesValidation(t *testing.T) {
	s := newTestServer(t)
	seed(t, s, "x", 1, 100)

	rec, _ := get(t, s, "/api/v1/favorites?offset=-1")
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("offset=-1 status = %d, want 400", rec.Code)
	}
	rec, _ = get(t, s, "/api/v1/favorites?limit=-1")
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("limit=-1 status = %d, want 400", rec.Code)
	}
	rec, _ = get(t, s, "/api/v1/favorites?offset=abc")
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("offset=abc status = %d, want 400", rec.Code)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/v1/favorites", nil)
	rec = httptest.NewRecorder()
	s.serveMux().ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("no token status = %d, want 401", rec.Code)
	}
}
