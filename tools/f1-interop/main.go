// Command f1-interop drives the reference host's appearance handlers and WebUI
// port settings, so files written by the native host can be read back by the
// reference code and vice versa (F-1 interop evidence).
//
//	f1-interop -root <dir> -method GET -path /api/v1/appearance/text-color [-body '<json>']
//	f1-interop -root <dir> -method POST -path /api/v1/appearance/panel-background/upload -upload image.png
//	f1-interop -root <dir> -webui-port-default 9099      # prints the port the reference would listen on
//
// The status line is printed first, then the body. For an image response only
// the content type, cache header and a SHA256 of the body are printed.
package main

import (
	"bytes"
	"crypto/sha256"
	"flag"
	"fmt"
	"mime/multipart"
	"net/http/httptest"
	"os"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/go-chi/chi/v5"
)

func main() {
	root := flag.String("root", "", "configuration root (MainConfigBaseDir)")
	method := flag.String("method", "GET", "HTTP method")
	path := flag.String("path", "", "request path")
	body := flag.String("body", "", "request body")
	upload := flag.String("upload", "", "file to send as the multipart field 'file'")
	portDefault := flag.Int("webui-port-default", 0, "print ResolveWebUIPortOverride(default)")
	flag.Parse()
	if *root == "" {
		flag.Usage()
		os.Exit(2)
	}
	coremain.MainConfigBaseDir = *root
	if *portDefault != 0 {
		fmt.Println(coremain.ResolveWebUIPortOverride(*portDefault))
		return
	}
	if *path == "" {
		flag.Usage()
		os.Exit(2)
	}
	router := chi.NewRouter()
	coremain.RegisterAppearanceAPI(router)

	var request = httptest.NewRequest(*method, *path, strings.NewReader(*body))
	if *upload != "" {
		data, err := os.ReadFile(*upload)
		if err != nil {
			fatal(err)
		}
		var buffer bytes.Buffer
		writer := multipart.NewWriter(&buffer)
		part, err := writer.CreateFormFile("file", "upload.bin")
		if err != nil {
			fatal(err)
		}
		_, _ = part.Write(data)
		_ = writer.Close()
		request = httptest.NewRequest(*method, *path, &buffer)
		request.Header.Set("Content-Type", writer.FormDataContentType())
	}
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)
	fmt.Println(recorder.Code)
	contentType := recorder.Header().Get("Content-Type")
	if strings.HasPrefix(contentType, "image/") || contentType == "application/octet-stream" {
		fmt.Printf("%s %s sha256=%x\n", contentType, recorder.Header().Get("Cache-Control"), sha256.Sum256(recorder.Body.Bytes()))
		return
	}
	fmt.Print(recorder.Body.String())
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
