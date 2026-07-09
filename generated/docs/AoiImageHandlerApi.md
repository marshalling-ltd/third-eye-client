# \AoiImageHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**aoi_image_list_handler**](AoiImageHandlerApi.md#aoi_image_list_handler) | **GET** /api/v1/aoi-images | 
[**aoi_image_list_search_handler**](AoiImageHandlerApi.md#aoi_image_list_search_handler) | **GET** /api/v1/aoi-images/search | 
[**delete_aoi_image_handler**](AoiImageHandlerApi.md#delete_aoi_image_handler) | **DELETE** /api/v1/aoi-images/{id} | 
[**get_aoi_image_handler**](AoiImageHandlerApi.md#get_aoi_image_handler) | **GET** /api/v1/aoi-images/{id} | 
[**request_aoi_image_analysis_handler**](AoiImageHandlerApi.md#request_aoi_image_analysis_handler) | **POST** /api/v1/aoi-images/{id}/request-analysis | 
[**update_aoi_image_analysis_handler**](AoiImageHandlerApi.md#update_aoi_image_analysis_handler) | **PATCH** /api/v1/aoi-images/{id} | 



## aoi_image_list_handler

> models::PagedResponseAoiImageModel aoi_image_list_handler(page, limit, order_by, desc, aoi_id, storage_secret_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**aoi_id** | Option<**uuid::Uuid**> | aoi ID |  |
**storage_secret_id** | Option<**uuid::Uuid**> | Storage secret ID: Aws, Google or Local |  |

### Return type

[**models::PagedResponseAoiImageModel**](PagedResponse_AoiImageModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## aoi_image_list_search_handler

> models::PagedResponseAoiImageWithAnalysisModel aoi_image_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by aoi id and/or storage secret ID |  |

### Return type

[**models::PagedResponseAoiImageWithAnalysisModel**](PagedResponse_AoiImageWithAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_aoi_image_handler

> delete_aoi_image_handler(id, delete_aoi_image_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi image id for deleting aoi | [required] |
**delete_aoi_image_schema** | [**DeleteAoiImageSchema**](DeleteAoiImageSchema.md) | Delete Aoi image options | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_aoi_image_handler

> models::AoiImageWithAnalysisModel get_aoi_image_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi image ID for fetching aoi | [required] |

### Return type

[**models::AoiImageWithAnalysisModel**](AoiImageWithAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## request_aoi_image_analysis_handler

> request_aoi_image_analysis_handler(id, update_aoi_image_options_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** |  | [required] |
**update_aoi_image_options_schema** | [**UpdateAoiImageOptionsSchema**](UpdateAoiImageOptionsSchema.md) | Update Aoi image options | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## update_aoi_image_analysis_handler

> models::AoiImageWithAnalysisModel update_aoi_image_analysis_handler(id, update_aoi_image_analysis_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** |  | [required] |
**update_aoi_image_analysis_schema** | [**UpdateAoiImageAnalysisSchema**](UpdateAoiImageAnalysisSchema.md) | Update AoiImage with Analysis | [required] |

### Return type

[**models::AoiImageWithAnalysisModel**](AoiImageWithAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

