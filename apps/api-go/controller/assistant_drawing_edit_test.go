/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package controller

import (
	"bytes"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func playgroundImageEditContext(t *testing.T, images [][]byte) *gin.Context {
	t.Helper()
	var body bytes.Buffer
	writer := multipart.NewWriter(&body)
	require.NoError(t, writer.WriteField("model", "gpt-image-1"))
	require.NoError(t, writer.WriteField("prompt", "Use image one and image two"))
	for index, image := range images {
		part, err := writer.CreateFormFile("image", "input-"+strconv.Itoa(index+1)+".png")
		require.NoError(t, err)
		_, err = part.Write(image)
		require.NoError(t, err)
	}
	require.NoError(t, writer.Close())
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = httptest.NewRequest(http.MethodPost, "/pg/images/edits", &body)
	c.Request.Header.Set("Content-Type", writer.FormDataContentType())
	return c
}

func TestParsePlaygroundImageEditFormSupportsMultipleImages(t *testing.T) {
	png := []byte("\x89PNG\r\n\x1a\n")
	c := playgroundImageEditContext(t, [][]byte{png, png})
	form, modelID, prompt, err := parsePlaygroundImageEditForm(c)
	require.NoError(t, err)
	defer form.RemoveAll()
	require.Equal(t, "gpt-image-1", modelID)
	require.Equal(t, "Use image one and image two", prompt)
	require.Len(t, form.File["image"], 2)
}

func TestParsePlaygroundImageEditFormRejectsUnsupportedFiles(t *testing.T) {
	c := playgroundImageEditContext(t, [][]byte{[]byte("not an image")})
	form, _, _, err := parsePlaygroundImageEditForm(c)
	require.ErrorContains(t, err, "PNG, JPEG, or WebP")
	defer form.RemoveAll()
}

func TestParsePlaygroundImageEditFormLimitsImageCount(t *testing.T) {
	png := []byte("\x89PNG\r\n\x1a\n")
	images := make([][]byte, assistantDrawingMaxReferences+1)
	for index := range images {
		images[index] = png
	}
	c := playgroundImageEditContext(t, images)
	form, _, _, err := parsePlaygroundImageEditForm(c)
	require.ErrorContains(t, err, "between 1 and 8")
	defer form.RemoveAll()
}
